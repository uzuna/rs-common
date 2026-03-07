use crate::error::Error;
use crate::frame::StorageFrame;
use crate::metadata::MetaDataHeader;
use crate::ProtectionLevel;
use serde::{de::DeserializeOwned, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;

/// 3つのスロットを用いたローリングアップデートによる永続化ストレージ。
///
/// 書き込み時は最も古いスロットを上書きし、読み込み時は破損していない最新のスロットを選択します。
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
/// let vault = ReliableVault::new("./data", "config");
///
/// let config = Config { version: 1, name: "device-001".to_string() };
/// vault.save(&config, ProtectionLevel::Medium).unwrap();
///
/// let loaded: Config = vault.load().unwrap();
/// ```
pub struct ReliableVault<T> {
    dir: PathBuf,
    filename_base: String,
    _phantom: std::marker::PhantomData<T>,
}

impl<T> ReliableVault<T>
where
    T: Serialize + DeserializeOwned,
{
    /// 新しいVaultを作成します。
    ///
    /// # Arguments
    /// * `dir`: データファイルを保存するディレクトリ。
    /// * `filename_base`: ファイル名のプレフィックス（例: "data" -> "data.0", "data.1", "data.2"）。
    pub fn new(dir: impl Into<PathBuf>, filename_base: impl Into<String>) -> Self {
        Self {
            dir: dir.into(),
            filename_base: filename_base.into(),
            _phantom: std::marker::PhantomData,
        }
    }

    fn slot_path(&self, index: usize) -> PathBuf {
        self.dir.join(format!("{}.{}", self.filename_base, index))
    }

    /// 最新の有効なデータを読み込みます。
    ///
    /// 全てのスロットを確認し、破損していないデータの中で最も新しいシーケンス番号を持つものを返します。
    pub fn load(&self) -> Result<T, Error> {
        let mut candidates = Vec::new();

        for i in 0..3 {
            let path = self.slot_path(i);
            if !path.exists() {
                continue;
            }

            // ファイル読み込み
            let bytes = match fs::read(&path) {
                Ok(b) => b,
                Err(_) => continue, // 読み込みエラー（権限など）はスキップ
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
    /// 現在の最大シーケンス番号を確認し、最も古いスロット（次のシーケンス番号 % 3）を上書きします。
    pub fn save(&self, data: &T, level: ProtectionLevel) -> Result<(), Error> {
        // ディレクトリ作成
        if !self.dir.exists() {
            fs::create_dir_all(&self.dir)?;
        }

        // 次のシーケンス番号を決定するためにヘッダーをスキャン
        let mut max_sequence = 0;
        let mut found_any = false;

        for i in 0..3 {
            let path = self.slot_path(i);
            if !path.exists() {
                continue;
            }

            let mut file = match File::open(&path) {
                Ok(f) => f,
                Err(_) => continue,
            };

            // Primary(16B) + Secondary(16B) = 32B だけ読んで高速にチェック
            let mut buf = [0u8; 32];
            if file.read_exact(&mut buf).is_ok() {
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
        let target_slot = (new_sequence % 3) as usize;
        let target_path = self.slot_path(target_slot);

        // ペイロードのシリアライズ
        let payload = bincode::serialize(data)?;

        // フレーム作成
        let frame = StorageFrame::new(payload, new_sequence, level);
        let bytes = frame.to_bytes()?;

        // 書き込み
        let mut file = File::create(target_path)?;
        file.write_all(&bytes)?;
        file.sync_all()?; // ディスクへのフラッシュを保証

        Ok(())
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

    #[test]
    fn test_vault_rotation_and_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let vault = ReliableVault::<TestData>::new(dir.path(), "test");

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
        corrupt_slot_footer_crc(&slot1);

        // 3. 自動フォールバック (次に新しい seq 3 が読み込まれるべき)
        let loaded_fallback = vault.load().unwrap();
        assert_eq!(
            loaded_fallback.id, 3,
            "破損した最新データの代わりに一つ前のデータが読み込まれるべき"
        );
    }

    #[test]
    fn test_vault_recovery_with_two_corrupted_slots() {
        let dir = tempfile::tempdir().unwrap();
        let vault = ReliableVault::<TestData>::new(dir.path(), "test");

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

    #[test]
    fn test_vault_load_fails_when_all_slots_corrupted() {
        let dir = tempfile::tempdir().unwrap();
        let vault = ReliableVault::<TestData>::new(dir.path(), "test");

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

    #[test]
    fn test_vault_recovery_with_truncated_latest_slot() {
        let dir = tempfile::tempdir().unwrap();
        let vault = ReliableVault::<TestData>::new(dir.path(), "test");

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

    #[test]
    fn test_vault_load_fails_when_all_slots_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let vault = ReliableVault::<TestData>::new(dir.path(), "test");

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

    #[test]
    fn test_vault_recovery_with_header_only_latest_slot() {
        let dir = tempfile::tempdir().unwrap();
        let vault = ReliableVault::<TestData>::new(dir.path(), "test");

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
            let vault = ReliableVault::<TestData>::new(dir.path(), "stress");

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
