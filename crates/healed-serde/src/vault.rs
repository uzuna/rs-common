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

        let result = vault.load();
        assert!(matches!(
            result,
            Err(Error::Io(ref e)) if e.kind() == std::io::ErrorKind::NotFound
        ));
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

        let result = vault.load();
        assert!(matches!(
            result,
            Err(Error::Io(ref e)) if e.kind() == std::io::ErrorKind::NotFound
        ));
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
}
