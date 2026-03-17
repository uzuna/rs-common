use crate::error::Error;
use crate::frame::StorageFrame;
use crate::metadata::MetaDataHeader;
use crate::tmr::{TmrStrategy, TMR_HEADER_GROUP_BYTES};
use crate::DataClass;
use crate::ProtectionLevel;
use bitflip::{BitFlipConfig, BitFlipWriter, BitFlipWriterError};
use serde::{de::DeserializeOwned, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const FRAME_HEADER_SCAN_BYTES: usize = 32;
const SLOT_SCAN_BYTES: usize = TMR_HEADER_GROUP_BYTES;

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

fn map_bitflip_writer_error(error: BitFlipWriterError) -> Error {
    match error {
        BitFlipWriterError::BitFlip(e) => Error::BitFlip(e),
        BitFlipWriterError::Io(e) => Error::Io(e),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SlotPayload {
    sequence: u64,
    payload: Vec<u8>,
    data_class: DataClass,
}

fn decode_slot_payload(bytes: &[u8]) -> Result<SlotPayload, Error> {
    if let Ok(slot) = TmrStrategy::decode_tmr_with_vote(bytes) {
        return Ok(SlotPayload {
            sequence: slot.sequence,
            payload: slot.payload,
            data_class: DataClass::Small,
        });
    }

    let frame = StorageFrame::recover(bytes)?;
    Ok(SlotPayload {
        sequence: frame.meta.sequence,
        payload: frame.payload,
        data_class: DataClass::Large,
    })
}

fn scan_slot_sequence(header_bytes: &[u8]) -> Option<u64> {
    TmrStrategy::peek_sequence(header_bytes).or_else(|| scan_frame_sequence(header_bytes))
}

fn scan_frame_sequence(header_bytes: &[u8]) -> Option<u64> {
    if header_bytes.len() < FRAME_HEADER_SCAN_BYTES {
        return None;
    }

    let primary_bytes: [u8; 16] = header_bytes[0..16].try_into().ok()?;
    if let Some(meta) = MetaDataHeader::from_bytes(&primary_bytes).decode() {
        return Some(meta.sequence);
    }

    let secondary_bytes: [u8; 16] = header_bytes[16..32].try_into().ok()?;
    MetaDataHeader::from_bytes(&secondary_bytes)
        .decode()
        .map(|meta| meta.sequence)
}

/// 標準のファイルシステムをバックエンドとして使用する実装。
pub struct FileSystemBackend {
    dir: PathBuf,
    filename_base: String,
    bitflip_config: Option<BitFlipConfig>,
}

impl FileSystemBackend {
    /// 新しい `FileSystemBackend` を作成します。
    pub fn new(dir: impl Into<PathBuf>, filename_base: impl Into<String>) -> Self {
        Self {
            dir: dir.into(),
            filename_base: filename_base.into(),
            bitflip_config: None,
        }
    }

    /// テスト/検証用途で保存時の故障注入を有効にします。
    pub fn with_bitflip_for_tests(mut self, config: BitFlipConfig) -> Self {
        self.bitflip_config = Some(config);
        self
    }

    fn slot_path(&self, index: usize) -> PathBuf {
        self.dir.join(format!("{}.{}", self.filename_base, index))
    }

    fn slot_temp_path(&self, index: usize) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        self.dir
            .join(format!("{}.{}.tmp-{}", self.filename_base, index, stamp))
    }

    fn sync_parent_dir(&self) -> Result<(), Error> {
        let dir = File::open(&self.dir)?;
        dir.sync_all()?;
        Ok(())
    }
}

impl StorageBackend for FileSystemBackend {
    fn read_slot(&self, index: usize) -> Result<Vec<u8>, Error> {
        fs::read(self.slot_path(index)).map_err(Into::into)
    }

    fn write_slot(&self, index: usize, data: &[u8]) -> Result<(), Error> {
        self.ensure_backend_exists()?;

        let slot_path = self.slot_path(index);
        let temp_path = self.slot_temp_path(index);
        let file = File::create(&temp_path)?;
        let file = match self.bitflip_config {
            Some(config) => {
                let mut writer = BitFlipWriter::new(file, config);
                writer.write_all(data)?;
                writer.finish().map_err(map_bitflip_writer_error)?
            }
            None => {
                let mut file = file;
                file.write_all(data)?;
                file
            }
        };
        if let Err(error) = file.sync_all() {
            let _ = fs::remove_file(&temp_path);
            return Err(error.into());
        }
        if let Err(error) = fs::rename(&temp_path, &slot_path) {
            let _ = fs::remove_file(&temp_path);
            return Err(error.into());
        }
        self.sync_parent_dir()?;
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
    bitflip_config: Option<BitFlipConfig>,
}

pub type InMemBackend = MemoryBackend;

impl MemoryBackend {
    /// 指定スロット数で新しい `MemoryBackend` を作成します。
    pub fn new(num_slots: usize) -> Self {
        assert!(num_slots > 0, "MemoryBackend requires at least 1 slot");
        Self {
            slots: Arc::new(Mutex::new(vec![None; num_slots])),
            bitflip_config: None,
        }
    }

    /// テスト/検証用途で保存時の故障注入を有効にします。
    pub fn with_bitflip_for_tests(mut self, config: BitFlipConfig) -> Self {
        self.bitflip_config = Some(config);
        self
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

    fn slot_ref(slots: &[Option<Vec<u8>>], index: usize) -> Result<&Option<Vec<u8>>, Error> {
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

        let stored = match self.bitflip_config {
            Some(config) => {
                let mut writer = BitFlipWriter::new(Vec::<u8>::new(), config);
                writer.write_all(data)?;
                writer.finish().map_err(map_bitflip_writer_error)?
            }
            None => data.to_vec(),
        };

        *slot = Some(stored);
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

    fn next_sequence(&self) -> u64 {
        let mut max_sequence = 0;
        let mut found_any = false;

        for index in 0..self.num_slots {
            let Ok(buffer) = self.backend.read_header(index, SLOT_SCAN_BYTES) else {
                continue;
            };

            if let Some(sequence) = scan_slot_sequence(&buffer) {
                max_sequence = max_sequence.max(sequence);
                found_any = true;
            }
        }

        if found_any {
            max_sequence + 1
        } else {
            1
        }
    }

    fn latest_slot(&self, expected_class: Option<DataClass>) -> Result<SlotPayload, Error> {
        let mut candidates = Vec::new();

        for index in 0..self.num_slots {
            let bytes = match self.backend.read_slot(index) {
                Ok(bytes) => bytes,
                Err(error) if error.is_recoverable() => continue,
                Err(error) => return Err(error),
            };

            let slot = match decode_slot_payload(&bytes) {
                Ok(slot) => slot,
                Err(error) if error.is_recoverable() => continue,
                Err(error) => return Err(error),
            };

            if expected_class.is_none_or(|class| slot.data_class == class) {
                candidates.push(slot);
            }
        }

        if candidates.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "No valid data slots found",
            )
            .into());
        }

        candidates.sort_by_key(|slot| std::cmp::Reverse(slot.sequence));
        Ok(candidates.remove(0))
    }

    /// 最新の有効なデータを読み込みます。
    ///
    /// 全てのスロットを確認し、破損していないデータの中で最も新しいシーケンス番号を持つものを返します。
    pub fn load(&self) -> Result<T, Error> {
        let latest = self.latest_slot(None)?;
        Ok(bincode::deserialize(&latest.payload)?)
    }

    /// 小サイズデータをTMRで保存します。
    pub fn save_small(&self, data: &T) -> Result<(), Error> {
        self.backend.ensure_backend_exists()?;

        let payload = bincode::serialize(data)?;
        if DataClass::from_payload_len(payload.len()) != DataClass::Small {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "Serialized payload size {} exceeds small-data threshold {}",
                    payload.len(),
                    crate::SMALL_DATA_THRESHOLD_BYTES
                ),
            )
            .into());
        }

        let sequence = self.next_sequence();
        let target_slot = (sequence as usize) % self.num_slots;
        let bytes = TmrStrategy::encode_tmr(sequence, &payload)?;
        self.backend.write_slot(target_slot, &bytes)
    }

    /// TMRで保存された最新の小サイズデータを読み込みます。
    pub fn load_small(&self) -> Result<T, Error> {
        let latest = self.latest_slot(Some(DataClass::Small))?;
        Ok(bincode::deserialize(&latest.payload)?)
    }

    /// データを保存します。
    ///
    /// 現在の最大シーケンス番号を確認し、最も古いスロット（次のシーケンス番号 % スロット数）を上書きします。
    pub fn save(&self, data: &T, level: ProtectionLevel) -> Result<(), Error> {
        self.backend.ensure_backend_exists()?;
        let new_sequence = self.next_sequence();
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
    use bitflip::{BitFlipConfig, BitFlipTarget};
    use serde::Deserialize;
    use std::path::Path;

    #[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
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

    fn count_diff_bits(lhs: &[u8], rhs: &[u8]) -> usize {
        assert_eq!(lhs.len(), rhs.len(), "length mismatch");
        lhs.iter()
            .zip(rhs.iter())
            .map(|(l, r)| (l ^ r).count_ones() as usize)
            .sum()
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

    /// BitFlip未設定時は保存データがそのまま保持されることを検証。
    #[test]
    fn test_backend_bitflip_disabled_by_default() {
        let backend = MemoryBackend::new(3);
        let payload = vec![0x5Au8; 64];

        backend.write_slot(0, &payload).unwrap();
        let stored = backend.read_slot(0).unwrap();

        assert_eq!(stored, payload);
    }

    /// Memory/FileSystem の両バックエンドでBitFlip注入が機能することを検証。
    #[test]
    fn test_backend_bitflip_injection_enabled_cases() {
        let payload = vec![0xA3u8; 64];
        let cases = [
            (
                "memory_backend",
                BitFlipConfig::new(5, Some(12345), BitFlipTarget::FullFrame),
            ),
            (
                "filesystem_backend",
                BitFlipConfig::new(5, Some(12345), BitFlipTarget::FullFrame),
            ),
        ];

        for (name, config) in cases {
            if name == "memory_backend" {
                let backend = MemoryBackend::new(3).with_bitflip_for_tests(config);
                backend
                    .write_slot(1, &payload)
                    .unwrap_or_else(|e| panic!("{name}: write_slot failed: {e}"));
                let stored = backend
                    .read_slot(1)
                    .unwrap_or_else(|e| panic!("{name}: read_slot failed: {e}"));

                assert_eq!(
                    count_diff_bits(&payload, &stored),
                    config.flip_bits,
                    "{name}: unexpected flipped bit count"
                );
                continue;
            }

            let dir = tempfile::tempdir().unwrap();
            let backend =
                FileSystemBackend::new(dir.path(), "bitflip").with_bitflip_for_tests(config);
            backend
                .write_slot(2, &payload)
                .unwrap_or_else(|e| panic!("{name}: write_slot failed: {e}"));
            let stored = backend
                .read_slot(2)
                .unwrap_or_else(|e| panic!("{name}: read_slot failed: {e}"));

            assert_eq!(
                count_diff_bits(&payload, &stored),
                config.flip_bits,
                "{name}: unexpected flipped bit count"
            );
        }
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

    /// Phase1の値域確認として、DataClass境界とエラー分類を検証。
    #[test]
    fn test_phase1_value_range() {
        let data_class_cases = [
            ("empty_payload", 0usize, crate::DataClass::Small),
            (
                "max_small_payload",
                crate::SMALL_DATA_THRESHOLD_BYTES - 1,
                crate::DataClass::Small,
            ),
            (
                "large_payload_boundary",
                crate::SMALL_DATA_THRESHOLD_BYTES,
                crate::DataClass::Large,
            ),
        ];

        for (name, payload_len, expected) in data_class_cases {
            let actual = crate::DataClass::from_payload_len(payload_len);
            assert_eq!(actual, expected, "{name}: DataClass mismatch");
        }

        let error_class_cases = [
            (
                "recoverable_crc",
                Error::CrcMismatch,
                crate::error::ErrorClass::Recoverable,
            ),
            (
                "fatal_invalid_protection_level",
                Error::InvalidProtectionLevel,
                crate::error::ErrorClass::Fatal,
            ),
            (
                "recoverable_not_found",
                Error::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "missing")),
                crate::error::ErrorClass::Recoverable,
            ),
        ];

        for (name, error, expected) in error_class_cases {
            assert_eq!(error.class(), expected, "{name}: error class mismatch");
        }
    }

    /// Phase1の正常系として、TMR小サイズ保存/復元と原子的書き込みを検証。
    #[test]
    fn test_phase1_ok_cases() {
        let small_data = TestData {
            id: 101,
            message: "small tmr payload".to_string(),
        };

        let memory_cases = [(
            "memory_save_small_roundtrip",
            MemoryBackend::new(3),
            small_data.clone(),
        )];

        for (name, backend, data) in memory_cases {
            let vault = ReliableVault::<TestData, _>::new(backend.clone(), 3);
            vault
                .save_small(&data)
                .unwrap_or_else(|e| panic!("{name}: save_small failed: {e}"));

            let loaded = vault
                .load_small()
                .unwrap_or_else(|e| panic!("{name}: load_small failed: {e}"));
            assert_eq!(loaded, data, "{name}: small roundtrip mismatch");

            backend
                .mutate_slot(1, |bytes| {
                    let payload_offset = crate::tmr::TMR_HEADER_GROUP_BYTES + 5;
                    bytes[payload_offset] ^= 0b0000_0010;
                })
                .unwrap_or_else(|e| panic!("{name}: mutate_slot failed: {e}"));

            let recovered = vault
                .load_small()
                .unwrap_or_else(|e| panic!("{name}: load_small after corruption failed: {e}"));
            assert_eq!(recovered, data, "{name}: TMR recovery mismatch");
        }

        let file_cases = [("filesystem_save_small_atomic", small_data.clone())];

        for (name, data) in file_cases {
            let dir = tempfile::tempdir().unwrap();
            let vault = ReliableVault::<TestData>::new_with_fs(dir.path(), "phase1");

            vault
                .save_small(&data)
                .unwrap_or_else(|e| panic!("{name}: save_small failed: {e}"));

            let loaded = vault
                .load_small()
                .unwrap_or_else(|e| panic!("{name}: load_small failed: {e}"));
            assert_eq!(loaded, data, "{name}: filesystem roundtrip mismatch");

            let mut entries = fs::read_dir(dir.path())
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            entries.sort();
            assert!(
                entries.iter().all(|entry| !entry.contains(".tmp-")),
                "{name}: temporary file should not remain: {entries:?}"
            );
        }
    }

    /// Phase1の異常系として、small APIのサイズ超過とsmall未存在時エラーを検証。
    #[test]
    fn test_phase1_error_cases() {
        #[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
        struct LargeCaseData {
            payload: Vec<u8>,
        }

        let too_large = LargeCaseData {
            payload: vec![0xEF; crate::SMALL_DATA_THRESHOLD_BYTES + 512],
        };
        let large_backend = MemoryBackend::new(3);
        let large_vault = ReliableVault::<LargeCaseData, _>::new(large_backend, 3);

        let save_small_error = match large_vault.save_small(&too_large) {
            Ok(_) => panic!("save_small should fail for large payload"),
            Err(error) => error,
        };
        assert!(
            matches!(
                save_small_error,
                Error::Io(ref error) if error.kind() == std::io::ErrorKind::InvalidInput
            ),
            "save_small large payload error mismatch: {save_small_error:?}"
        );

        let dir = tempfile::tempdir().unwrap();
        let large_only_vault = ReliableVault::<JsonCaseData>::new_with_fs(dir.path(), "large-only");
        let large_case = build_json_case_data(90, 2048, 16 * 1024);
        large_only_vault
            .save(&large_case, ProtectionLevel::Medium)
            .unwrap();

        let load_small_error = match large_only_vault.load_small() {
            Ok(_) => panic!("load_small should fail when only large slots exist"),
            Err(error) => error,
        };
        assert!(
            matches!(
                load_small_error,
                Error::Io(ref error) if error.kind() == std::io::ErrorKind::NotFound
            ),
            "load_small missing small slot error mismatch: {load_small_error:?}"
        );
    }
}
