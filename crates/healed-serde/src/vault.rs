use crate::error::Error;
use crate::frame::StorageFrame;
use crate::metadata::MetaDataHeader;
use crate::ProtectionLevel;
use serde::{de::DeserializeOwned, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// ストレージバックエンドの振る舞いを定義するトレイト。
///
/// これを実装することで、ファイルシステム以外のバックエンド（KVS、mmapなど）を
/// `ReliableVault` で使用できます。
pub trait StorageBackend {
    /// 指定されたインデックスのスロットからデータをバイト列として読み込みます。
    /// スロットが存在しない場合は `std::io::ErrorKind::NotFound` を含む `Error::Io` を返すべきです。
    fn read_slot(&self, index: usize) -> Result<Vec<u8>, Error>;

    /// 指定されたインデックスのスロットにデータを書き込みます。
    fn write_slot(&self, index: usize, data: &[u8]) -> Result<(), Error>;

    /// スロットの先頭から指定された長さのデータを読み込みます。
    /// `save` 時のシーケンス番号スキャンを高速化するために使用されます。
    fn read_header(&self, index: usize, len: usize) -> Result<Vec<u8>, Error>;

    /// バックエンド（ディレクトリなど）が存在することを保証します。
    fn ensure_backend_exists(&self) -> Result<(), Error>;
}

/// 標準のファイルシステムをバックエンドとして使用する実装。
pub struct FileSystemBackend {
    dir: PathBuf,
    filename_base: String,
}

impl FileSystemBackend {
    /// 新しい `FileSystemBackend` を作成します。
    pub fn new(dir: impl Into<PathBuf>, filename_base: impl Into<String>) -> Self {
        Self {
            dir: dir.into(),
            filename_base: filename_base.into(),
        }
    }

    fn slot_path(&self, index: usize) -> PathBuf {
        self.dir.join(format!("{}.{}", self.filename_base, index))
    }
}

impl StorageBackend for FileSystemBackend {
    fn read_slot(&self, index: usize) -> Result<Vec<u8>, Error> {
        fs::read(self.slot_path(index)).map_err(Into::into)
    }

    fn write_slot(&self, index: usize, data: &[u8]) -> Result<(), Error> {
        let mut file = File::create(self.slot_path(index))?;
        file.write_all(data)?;
        file.sync_all()?;
        Ok(())
    }

    fn read_header(&self, index: usize, len: usize) -> Result<Vec<u8>, Error> {
        let file = File::open(self.slot_path(index))?;
        let mut buf = Vec::with_capacity(len);
        file.take(len as u64).read_to_end(&mut buf)?;
        Ok(buf)
    }

    fn ensure_backend_exists(&self) -> Result<(), Error> {
        if !self.dir.exists() {
            fs::create_dir_all(&self.dir)?;
        }
        Ok(())
    }
}

/// メモリ領域のみを使用する `StorageBackend` 実装。
///
/// テスト用途や、プロセス寿命内だけの一時ストレージ用途を想定しています。
#[derive(Debug, Clone)]
pub struct MemoryBackend {
    slots: Arc<Mutex<Vec<Option<Vec<u8>>>>>,
}

impl MemoryBackend {
    /// 指定スロット数で新しい `MemoryBackend` を作成します。
    pub fn new(num_slots: usize) -> Self {
        assert!(num_slots > 0, "MemoryBackend requires at least 1 slot");
        Self {
            slots: Arc::new(Mutex::new(vec![None; num_slots])),
        }
    }

    fn lock_slots(&self) -> Result<std::sync::MutexGuard<'_, Vec<Option<Vec<u8>>>>, Error> {
        self.slots
            .lock()
            .map_err(|_| std::io::Error::other("MemoryBackend lock poisoned").into())
    }

    fn not_found(index: usize) -> Error {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Memory slot {index} not found"),
        )
        .into()
    }

    fn invalid_index(index: usize) -> Error {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("Memory slot index {index} is out of bounds"),
        )
        .into()
    }

    fn slot_ref(
        slots: &[Option<Vec<u8>>],
        index: usize,
    ) -> Result<&Option<Vec<u8>>, Error> {
        slots.get(index).ok_or_else(|| Self::invalid_index(index))
    }

    fn slot_mut(
        slots: &mut [Option<Vec<u8>>],
        index: usize,
    ) -> Result<&mut Option<Vec<u8>>, Error> {
        slots
            .get_mut(index)
            .ok_or_else(|| Self::invalid_index(index))
    }

    #[cfg(test)]
    fn mutate_slot<F>(&self, index: usize, mutator: F) -> Result<(), Error>
    where
        F: FnOnce(&mut Vec<u8>),
    {
        let mut slots = self.lock_slots()?;
        let slot = Self::slot_mut(&mut slots, index)?;
        let data = slot.as_mut().ok_or_else(|| Self::not_found(index))?;
        mutator(data);
        Ok(())
    }
}

impl StorageBackend for MemoryBackend {
    fn read_slot(&self, index: usize) -> Result<Vec<u8>, Error> {
        let slots = self.lock_slots()?;
        let slot = Self::slot_ref(&slots, index)?;
        slot.clone().ok_or_else(|| Self::not_found(index))
    }

    fn write_slot(&self, index: usize, data: &[u8]) -> Result<(), Error> {
        let mut slots = self.lock_slots()?;
        let slot = Self::slot_mut(&mut slots, index)?;
        *slot = Some(data.to_vec());
        Ok(())
    }

    fn read_header(&self, index: usize, len: usize) -> Result<Vec<u8>, Error> {
        let mut data = self.read_slot(index)?;
        if data.len() > len {
            data.truncate(len);
        }
        Ok(data)
    }

    fn ensure_backend_exists(&self) -> Result<(), Error> {
        Ok(())
    }
}

/// 複数のスロットを用いたローリングアップデートによる永続化ストレージ。
///
/// 書き込み時は最も古いスロットを上書きし、読み込み時は破損していない最新のスロットを選択します。
/// ストレージバックエンドは `StorageBackend` トレイトを介して抽象化されています。
///
/// # Examples
///
/// ```no_run
/// use healed_serde::vault::ReliableVault;
/// use healed_serde::ProtectionLevel;
/// use serde::{Serialize, Deserialize};
///
/// #[derive(Serialize, Deserialize, PartialEq, Debug)]
/// struct Config {
///     version: u32,
///     name: String,
/// }
///
/// // ファイルシステムをバックエンドとして使用する例
/// let vault = ReliableVault::<_>::new_with_fs("./data", "config");
///
/// let config = Config { version: 1, name: "device-001".to_string() };
/// vault.save(&config, ProtectionLevel::Medium).unwrap();
///
/// let loaded: Config = vault.load().unwrap();
/// ```
pub struct ReliableVault<T, B: StorageBackend = FileSystemBackend> {
    backend: B,
    num_slots: usize,
    _phantom: std::marker::PhantomData<T>,
}

impl<T> ReliableVault<T, FileSystemBackend>
where
    T: Serialize + DeserializeOwned,
{
    /// ファイルシステムをバックエンドとして使用する新しいVaultを3スロットで作成します。
    ///
    /// # Arguments
    /// * `dir`: データファイルを保存するディレクトリ。
    /// * `filename_base`: ファイル名のプレフィックス（例: "data" -> "data.0", "data.1", "data.2"）。
    pub fn new_with_fs(dir: impl Into<PathBuf>, filename_base: impl Into<String>) -> Self {
        let backend = FileSystemBackend::new(dir, filename_base);
        Self::new(backend, 3)
    }
}

impl<T, B: StorageBackend> ReliableVault<T, B>
where
    T: Serialize + DeserializeOwned,
{
    /// 指定されたバックエンドとスロット数で新しいVaultを作成します。
    pub fn new(backend: B, num_slots: usize) -> Self {
        assert!(num_slots >= 3, "ReliableVault requires at least 3 slots");
        Self {
            backend,
            num_slots,
            _phantom: std::marker::PhantomData,
        }
    }

    /// 最新の有効なデータを読み込みます。
    ///
    /// 全てのスロットを確認し、破損していないデータの中で最も新しいシーケンス番号を持つものを返します。
    pub fn load(&self) -> Result<T, Error> {
        let mut candidates = Vec::new();

        for i in 0..self.num_slots {
            let bytes = match self.backend.read_slot(i) {
                Ok(b) => b,
                Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => continue, // その他の読み込みエラーはスキップ
            };

            // フレーム復元試行
            match StorageFrame::recover(&bytes) {
                Ok(frame) => {
                    candidates.push(frame);
                }
                Err(_) => continue, // 破損データはスキップ
            }
        }

        if candidates.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "No valid data slots found",
            )
            .into());
        }

        // シーケンス番号で降順ソート (最新が先頭)
        candidates.sort_by_key(|f| std::cmp::Reverse(f.meta.sequence));

        // 最新のものをデシリアライズ
        let latest = &candidates[0];
        let data: T = bincode::deserialize(&latest.payload)?;

        Ok(data)
    }

    /// データを保存します。
    ///
    /// 現在の最大シーケンス番号を確認し、最も古いスロット（次のシーケンス番号 % スロット数）を上書きします。
    pub fn save(&self, data: &T, level: ProtectionLevel) -> Result<(), Error> {
        self.backend.ensure_backend_exists()?;

        // 次のシーケンス番号を決定するためにヘッダーをスキャン
        let mut max_sequence = 0;
        let mut found_any = false;

        for i in 0..self.num_slots {
            if let Ok(buf) = self.backend.read_header(i, 32) {
                if buf.len() != 32 {
                    continue;
                }

                // Primary Header check
                let primary_bytes: [u8; 16] = buf[0..16].try_into().unwrap();
                if let Some(meta) = MetaDataHeader::from_bytes(&primary_bytes).decode() {
                    if meta.sequence > max_sequence {
                        max_sequence = meta.sequence;
                    }
                    found_any = true;
                    continue; // Primaryが生きていればSecondaryは見なくてよい
                }

                // Secondary Header check
                let secondary_bytes: [u8; 16] = buf[16..32].try_into().unwrap();
                if let Some(meta) = MetaDataHeader::from_bytes(&secondary_bytes).decode() {
                    if meta.sequence > max_sequence {
                        max_sequence = meta.sequence;
                    }
                    found_any = true;
                }
            }
        }

        let new_sequence = if found_any { max_sequence + 1 } else { 1 };
        let target_slot = (new_sequence as usize) % self.num_slots;

        // ペイロードのシリアライズ
        let payload = bincode::serialize(data)?;

        // フレーム作成
        let frame = StorageFrame::new(payload, new_sequence, level);
        let bytes = frame.to_bytes()?;

        self.backend.write_slot(target_slot, &bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::path::Path;

    #[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
    struct TestData {
        id: u32,
        message: String,
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
    struct JsonCaseData {
        id: u64,
        name: String,
        tags: Vec<String>,
        values: Vec<u64>,
        payload: String,
    }

    /// ストレステストで注入する故障シナリオ。
    #[derive(Debug, Clone, Copy)]
    enum StressScenario {
        LatestFooterCrcCorrupt,
        LatestTruncated,
        LatestHeaderOnly,
        TwoLatestFooterCrcCorrupt,
        LatestHeaderOnlyAndSecondCorrupt,
        AllFooterCrcCorrupt,
        AllTruncated,
        AllHeaderOnly,
    }

    /// シナリオ実行後に期待される読み出し結果。
    #[derive(Debug, Clone, Copy)]
    enum ExpectedOutcome {
        LoadedId(u32),
        NotFound,
    }

    fn corrupt_slot_footer_crc(path: &Path) {
        let mut bytes = fs::read(path).unwrap();
        let crc_offset = bytes.len() - 8;
        bytes[crc_offset] ^= 0xFF;
        fs::write(path, bytes).unwrap();
    }

    fn truncate_slot(path: &Path, size: usize) {
        let mut bytes = fs::read(path).unwrap();
        bytes.truncate(size);
        fs::write(path, bytes).unwrap();
    }

    fn assert_not_found(result: Result<TestData, Error>) {
        assert!(matches!(
            result,
            Err(Error::Io(ref e)) if e.kind() == std::io::ErrorKind::NotFound
        ));
    }

    fn build_json_case_data(id: u64, values_len: usize, payload_len: usize) -> JsonCaseData {
        JsonCaseData {
            id,
            name: format!("case-{id}"),
            tags: (0..8).map(|i| format!("tag-{id}-{i}")).collect(),
            values: (0..values_len).map(|i| (id + i as u64) * 3).collect(),
            payload: "x".repeat(payload_len),
        }
    }

    /// MemoryBackendの基本I/O (read/write/read_header) と NotFound を検証。
    #[test]
    fn test_memory_backend_read_write_and_header() {
        let backend = MemoryBackend::new(3);
        backend.ensure_backend_exists().unwrap();

        let payload = vec![1u8, 2, 3, 4, 5];
        backend.write_slot(1, &payload).unwrap();

        let full = backend.read_slot(1).unwrap();
        assert_eq!(full, payload);

        let header = backend.read_header(1, 3).unwrap();
        assert_eq!(header, vec![1u8, 2, 3]);

        assert!(matches!(
            backend.read_slot(0),
            Err(Error::Io(ref e)) if e.kind() == std::io::ErrorKind::NotFound
        ));
    }

    /// MemoryBackendの境界外インデックスが InvalidInput として扱われることを検証。
    #[test]
    fn test_memory_backend_boundary_invalid_index() {
        let backend = MemoryBackend::new(3);

        assert!(matches!(
            backend.read_slot(3),
            Err(Error::Io(ref e)) if e.kind() == std::io::ErrorKind::InvalidInput
        ));
        assert!(matches!(
            backend.read_header(99, 8),
            Err(Error::Io(ref e)) if e.kind() == std::io::ErrorKind::InvalidInput
        ));
        assert!(matches!(
            backend.write_slot(10, &[1, 2, 3]),
            Err(Error::Io(ref e)) if e.kind() == std::io::ErrorKind::InvalidInput
        ));
    }

    /// read_header の長さ0指定で空データが返ることを検証。
    #[test]
    fn test_memory_backend_boundary_zero_header_len() {
        let backend = MemoryBackend::new(3);
        backend.write_slot(0, &[1, 2, 3, 4]).unwrap();

        let header = backend.read_header(0, 0).unwrap();
        assert!(header.is_empty());
    }

    /// スロット数0のMemoryBackend生成は不正なためpanicすることを検証。
    #[test]
    #[should_panic(expected = "MemoryBackend requires at least 1 slot")]
    fn test_memory_backend_new_panics_with_zero_slots() {
        let _ = MemoryBackend::new(0);
    }

    /// ReliableVaultは3スロット以上前提のため、2スロット指定でpanicすることを検証。
    #[test]
    #[should_panic(expected = "ReliableVault requires at least 3 slots")]
    fn test_vault_new_panics_when_slots_less_than_three() {
        let backend = MemoryBackend::new(2);
        let _ = ReliableVault::<TestData, _>::new(backend, 2);
    }

    /// MemoryBackend上でsave/loadの往復が成立することを検証。
    #[test]
    fn test_vault_with_memory_backend_roundtrip() {
        let backend = MemoryBackend::new(3);
        let vault = ReliableVault::<TestData, _>::new(backend, 3);

        for i in 1..=4 {
            let data = TestData {
                id: i,
                message: format!("msg {}", i),
            };
            vault.save(&data, ProtectionLevel::Medium).unwrap();
        }

        let loaded = vault.load().unwrap();
        assert_eq!(loaded.id, 4);
    }

    /// MemoryBackend上で最新スロット破損時に次点スロットへフォールバックできることを検証。
    #[test]
    fn test_vault_with_memory_backend_fallback_after_corruption() {
        let backend = MemoryBackend::new(3);
        let vault = ReliableVault::<TestData, _>::new(backend.clone(), 3);

        // seq: 1->slot1, 2->slot2, 3->slot0, 4->slot1
        for i in 1..=4 {
            let data = TestData {
                id: i,
                message: format!("msg {}", i),
            };
            vault.save(&data, ProtectionLevel::High).unwrap();
        }

        backend
            .mutate_slot(1, |bytes| {
                let crc_pos = bytes.len() - 8;
                bytes[crc_pos] ^= 0xFF;
            })
            .unwrap();

        let loaded = vault.load().unwrap();
        assert_eq!(loaded.id, 3);
    }

    /// 5スロット運用時のローテーション境界と複数スロット破損後のフォールバックを検証。
    #[test]
    fn test_vault_with_memory_backend_boundary_rotation_five_slots() {
        let backend = MemoryBackend::new(5);
        let vault = ReliableVault::<TestData, _>::new(backend.clone(), 5);

        // seq1..7 を保存して、5スロット循環の境界を跨ぐ
        for i in 1..=7 {
            let data = TestData {
                id: i,
                message: format!("msg {}", i),
            };
            vault.save(&data, ProtectionLevel::Medium).unwrap();
        }

        // 最新は seq7
        let loaded_latest = vault.load().unwrap();
        assert_eq!(loaded_latest.id, 7);

        // seq7(slot2) と seq6(slot1) を壊すと次点は seq5(slot0)
        backend
            .mutate_slot(2, |bytes| {
                let crc_pos = bytes.len() - 8;
                bytes[crc_pos] ^= 0xFF;
            })
            .unwrap();
        backend
            .mutate_slot(1, |bytes| {
                let crc_pos = bytes.len() - 8;
                bytes[crc_pos] ^= 0xFF;
            })
            .unwrap();

        let loaded_fallback = vault.load().unwrap();
        assert_eq!(loaded_fallback.id, 5);
    }

    /// 最新スロット破損時に1世代前へフォールバックできることを検証。
    #[test]
    fn test_vault_rotation_and_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let vault = ReliableVault::<TestData>::new_with_fs(dir.path(), "test");

        // 1. 正常なローテーション (4回保存 -> seq 1, 2, 3, 4)
        // slot mapping: 1->1, 2->2, 3->0, 4->1(overwrite)
        for i in 1..=4 {
            let data = TestData {
                id: i,
                message: format!("msg {}", i),
            };
            vault.save(&data, ProtectionLevel::Medium).unwrap();
        }

        let loaded = vault.load().unwrap();
        assert_eq!(loaded.id, 4, "最新のデータ(id=4)が読み込まれるべき");

        // 2. 最新データの破損 (slot 1, seq 4 を破壊)
        let slot1 = dir.path().join("test.1");
        let mut bytes = fs::read(&slot1).unwrap();
        let crc_pos = bytes.len() - 8;
        bytes[crc_pos] ^= 0xFF; // CRCを破壊
        fs::write(&slot1, bytes).unwrap();

        // 3. 自動フォールバック (次に新しい seq 3 が読み込まれるべき)
        let loaded_fallback = vault.load().unwrap();
        assert_eq!(
            loaded_fallback.id, 3,
            "破損した最新データの代わりに一つ前のデータが読み込まれるべき"
        );
    }

    /// 最新2スロット破損時にさらに古い健全スロットへフォールバックできることを検証。
    #[test]
    fn test_vault_recovery_with_two_corrupted_slots() {
        let dir = tempfile::tempdir().unwrap();
        let vault = ReliableVault::<TestData>::new_with_fs(dir.path(), "test");

        // seq: 1->slot1, 2->slot2, 3->slot0, 4->slot1, 5->slot2
        // 最終状態: slot0=seq3, slot1=seq4, slot2=seq5
        for i in 1..=5 {
            let data = TestData {
                id: i,
                message: format!("msg {}", i),
            };
            vault.save(&data, ProtectionLevel::Medium).unwrap();
        }

        // 最新2スロットを破損させる
        corrupt_slot_footer_crc(&dir.path().join("test.2")); // seq5
        corrupt_slot_footer_crc(&dir.path().join("test.1")); // seq4

        // 残存する有効なseq3へフォールバックできること
        let loaded = vault.load().unwrap();
        assert_eq!(loaded.id, 3);
    }

    /// 全スロット破損時にNotFound相当エラーとなることを検証。
    #[test]
    fn test_vault_load_fails_when_all_slots_corrupted() {
        let dir = tempfile::tempdir().unwrap();
        let vault = ReliableVault::<TestData>::new_with_fs(dir.path(), "test");

        for i in 1..=3 {
            let data = TestData {
                id: i,
                message: format!("msg {}", i),
            };
            vault.save(&data, ProtectionLevel::Low).unwrap();
        }

        for slot in 0..3 {
            corrupt_slot_footer_crc(&dir.path().join(format!("test.{}", slot)));
        }

        assert_not_found(vault.load());
    }

    /// 最新スロットが途中書き込み（truncate）でも次点へフォールバックできることを検証。
    #[test]
    fn test_vault_recovery_with_truncated_latest_slot() {
        let dir = tempfile::tempdir().unwrap();
        let vault = ReliableVault::<TestData>::new_with_fs(dir.path(), "test");

        // seq: 1->slot1, 2->slot2, 3->slot0, 4->slot1
        for i in 1..=4 {
            let data = TestData {
                id: i,
                message: format!("msg {}", i),
            };
            vault.save(&data, ProtectionLevel::Medium).unwrap();
        }

        // 最新slot(seq4)を途中書き込み想定で短くする
        truncate_slot(&dir.path().join("test.1"), 12);

        // 有効な次点seq3にフォールバックできること
        let loaded = vault.load().unwrap();
        assert_eq!(loaded.id, 3);
    }

    /// 全スロットが途中書き込み状態（truncate）の場合にNotFoundとなることを検証。
    #[test]
    fn test_vault_load_fails_when_all_slots_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let vault = ReliableVault::<TestData>::new_with_fs(dir.path(), "test");

        for i in 1..=3 {
            let data = TestData {
                id: i,
                message: format!("msg {}", i),
            };
            vault.save(&data, ProtectionLevel::High).unwrap();
        }

        // 3スロットすべて途中書き込み相当の短い不完全ファイルにする
        for slot in 0..3 {
            truncate_slot(&dir.path().join(format!("test.{}", slot)), 8);
        }

        assert_not_found(vault.load());
    }

    /// 最新スロットがヘッダーのみ（32B）でも次点へフォールバックできることを検証。
    #[test]
    fn test_vault_recovery_with_header_only_latest_slot() {
        let dir = tempfile::tempdir().unwrap();
        let vault = ReliableVault::<TestData>::new_with_fs(dir.path(), "test");

        // seq: 1->slot1, 2->slot2, 3->slot0, 4->slot1
        for i in 1..=4 {
            let data = TestData {
                id: i,
                message: format!("msg {}", i),
            };
            vault.save(&data, ProtectionLevel::Medium).unwrap();
        }

        // 最新slot(seq4)をヘッダーのみ(Primary+Secondary=32B)に切り詰める
        truncate_slot(&dir.path().join("test.1"), 32);

        // 不完全ファイルを無視して、有効な次点seq3へフォールバックできること
        let loaded = vault.load().unwrap();
        assert_eq!(loaded.id, 3);
    }

    /// 代表的な破損シナリオを列挙し、期待結果（復旧 or NotFound）を網羅的に検証。
    #[test]
    fn test_vault_stress_cases_enumerated() {
        let cases = [
            (
                "latest_footer_crc_corrupt",
                StressScenario::LatestFooterCrcCorrupt,
                ExpectedOutcome::LoadedId(4),
            ),
            (
                "latest_truncated",
                StressScenario::LatestTruncated,
                ExpectedOutcome::LoadedId(4),
            ),
            (
                "latest_header_only",
                StressScenario::LatestHeaderOnly,
                ExpectedOutcome::LoadedId(4),
            ),
            (
                "two_latest_footer_crc_corrupt",
                StressScenario::TwoLatestFooterCrcCorrupt,
                ExpectedOutcome::LoadedId(3),
            ),
            (
                "latest_header_only_and_second_corrupt",
                StressScenario::LatestHeaderOnlyAndSecondCorrupt,
                ExpectedOutcome::LoadedId(3),
            ),
            (
                "all_footer_crc_corrupt",
                StressScenario::AllFooterCrcCorrupt,
                ExpectedOutcome::NotFound,
            ),
            (
                "all_truncated",
                StressScenario::AllTruncated,
                ExpectedOutcome::NotFound,
            ),
            (
                "all_header_only",
                StressScenario::AllHeaderOnly,
                ExpectedOutcome::NotFound,
            ),
        ];

        for (name, scenario, expected) in cases {
            let dir = tempfile::tempdir().unwrap();
            let vault = ReliableVault::<TestData>::new_with_fs(dir.path(), "stress");

            for id in 1..=5 {
                let data = TestData {
                    id,
                    message: format!("msg {}", id),
                };
                vault.save(&data, ProtectionLevel::Medium).unwrap();
            }

            let slot0 = dir.path().join("stress.0"); // seq3
            let slot1 = dir.path().join("stress.1"); // seq4
            let slot2 = dir.path().join("stress.2"); // seq5

            match scenario {
                StressScenario::LatestFooterCrcCorrupt => corrupt_slot_footer_crc(&slot2),
                StressScenario::LatestTruncated => truncate_slot(&slot2, 12),
                StressScenario::LatestHeaderOnly => truncate_slot(&slot2, 32),
                StressScenario::TwoLatestFooterCrcCorrupt => {
                    corrupt_slot_footer_crc(&slot2);
                    corrupt_slot_footer_crc(&slot1);
                }
                StressScenario::LatestHeaderOnlyAndSecondCorrupt => {
                    truncate_slot(&slot2, 32);
                    corrupt_slot_footer_crc(&slot1);
                }
                StressScenario::AllFooterCrcCorrupt => {
                    corrupt_slot_footer_crc(&slot0);
                    corrupt_slot_footer_crc(&slot1);
                    corrupt_slot_footer_crc(&slot2);
                }
                StressScenario::AllTruncated => {
                    truncate_slot(&slot0, 8);
                    truncate_slot(&slot1, 8);
                    truncate_slot(&slot2, 8);
                }
                StressScenario::AllHeaderOnly => {
                    truncate_slot(&slot0, 32);
                    truncate_slot(&slot1, 32);
                    truncate_slot(&slot2, 32);
                }
            }

            match expected {
                ExpectedOutcome::LoadedId(expected_id) => {
                    let loaded = vault
                        .load()
                        .unwrap_or_else(|e| panic!("{name}: load failed unexpectedly: {e}"));
                    assert_eq!(loaded.id, expected_id, "{name}");
                }
                ExpectedOutcome::NotFound => {
                    let result = vault.load();
                    assert!(
                        matches!(
                            result,
                            Err(Error::Io(ref e)) if e.kind() == std::io::ErrorKind::NotFound
                        ),
                        "{name}: expected NotFound, got {result:?}"
                    );
                }
            }
        }
    }

    /// JSONシリアライズのサイズ・復元整合性を small/medium/large ケースで検証。
    #[test]
    fn test_json_serialize_cases() {
        let cases = [
            ("small", build_json_case_data(1, 32, 256)),
            ("medium", build_json_case_data(2, 512, 4 * 1024)),
            ("large", build_json_case_data(3, 4096, 32 * 1024)),
        ];

        let mut prev_size = 0usize;
        for (name, case) in cases {
            let encoded = serde_json::to_vec(&case)
                .unwrap_or_else(|e| panic!("{name}: failed to serialize JSON: {e}"));
            assert!(
                !encoded.is_empty(),
                "{name}: serialized JSON should not be empty"
            );
            assert!(
                encoded.len() > prev_size,
                "{name}: serialized JSON size should grow with input size"
            );

            let decoded: JsonCaseData = serde_json::from_slice(&encoded)
                .unwrap_or_else(|e| panic!("{name}: failed to deserialize JSON: {e}"));
            assert_eq!(decoded, case, "{name}: JSON roundtrip mismatch");

            prev_size = encoded.len();
        }
    }
}
