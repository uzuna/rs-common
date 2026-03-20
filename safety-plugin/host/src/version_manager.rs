//! プラグインのバージョンを管理するモジュール。
//!
//! .so バイナリをストレージディレクトリに保存し、ロールバック用に最大 N 世代保持する。
//! バージョンは 1 から始まる自動連番で管理する。

use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;

/// プラグインの1バージョン分の情報。
#[derive(Debug, Clone)]
pub struct PluginVersion {
    /// バージョン番号（1から始まる連番）。
    pub version: u64,
    /// この .so ファイルのパス。
    pub path: PathBuf,
    /// 保存した UNIX 時刻（秒）。
    pub saved_at: u64,
}

/// プラグインのバージョンを管理する。
pub struct VersionManager {
    versions: Vec<PluginVersion>,
    next_version: u64,
    storage_dir: PathBuf,
    max_versions: usize,
    /// HTTP API 経由でロードした際の現在バージョン番号。
    /// ファイル監視経由のリロードでは None のまま。
    current: Option<u64>,
}

impl VersionManager {
    /// 新しい `VersionManager` を作成する。ストレージディレクトリが存在しない場合は作成する。
    pub fn new(storage_dir: PathBuf, max_versions: usize) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&storage_dir).with_context(|| {
            format!(
                "バージョンストレージディレクトリの作成に失敗: {}",
                storage_dir.display()
            )
        })?;
        Ok(Self {
            versions: Vec::new(),
            next_version: 1,
            storage_dir,
            max_versions,
            current: None,
        })
    }

    /// .so バイナリを保存し、割り当てたバージョン番号を返す。
    ///
    /// 保存成功後に `current` を更新する。古い世代が `max_versions` を超えた場合は削除する。
    pub fn save(&mut self, bytes: &[u8]) -> anyhow::Result<u64> {
        let version = self.next_version;
        let filename = format!("plugin_v{version}.so");
        let path = self.storage_dir.join(&filename);

        std::fs::write(&path, bytes)
            .with_context(|| format!("プラグインバイナリの保存に失敗: {}", path.display()))?;

        let saved_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        self.versions.push(PluginVersion {
            version,
            path,
            saved_at,
        });
        self.next_version += 1;
        self.current = Some(version);

        // 上限を超えた古いバージョンを削除する
        self.purge_old_versions();

        tracing::info!("プラグインを v{version} として保存");
        Ok(version)
    }

    /// 指定バージョンのファイルパスを返す。存在しない場合は `None`。
    pub fn path_of(&self, version: u64) -> Option<&Path> {
        self.versions
            .iter()
            .find(|v| v.version == version)
            .map(|v| v.path.as_path())
    }

    /// 現在アクティブなバージョン番号を設定する（ロールバック時に使用）。
    pub fn mark_current(&mut self, version: u64) {
        if self.versions.iter().any(|v| v.version == version) {
            self.current = Some(version);
        }
    }

    /// 現在アクティブなバージョン番号を返す。ファイル監視経由のリロード時は `None`。
    pub fn current_version(&self) -> Option<u64> {
        self.current
    }

    /// 管理中のバージョン一覧を返す（古い順）。
    pub fn list(&self) -> &[PluginVersion] {
        &self.versions
    }

    /// 上限を超えた古いバージョンを削除する。
    fn purge_old_versions(&mut self) {
        while self.versions.len() > self.max_versions {
            let old = self.versions.remove(0);
            if let Err(e) = std::fs::remove_file(&old.path) {
                tracing::warn!("古いバージョン (v{}) の削除に失敗: {e}", old.version);
            } else {
                tracing::debug!("古いバージョン (v{}) を削除", old.version);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp_manager(max: usize) -> (VersionManager, TempDir) {
        let dir = TempDir::new().unwrap();
        let mgr = VersionManager::new(dir.path().to_path_buf(), max).unwrap();
        (mgr, dir)
    }

    /// 正常系: save でバージョン番号が連番で割り当てられること。
    #[test]
    fn test_save_increments_version() {
        let (mut mgr, _dir) = tmp_manager(10);
        let cases = [(b"data1" as &[u8], 1u64), (b"data2", 2), (b"data3", 3)];
        for (data, expected) in cases {
            let v = mgr.save(data).unwrap();
            assert_eq!(v, expected);
        }
        assert_eq!(mgr.list().len(), 3);
    }

    /// 正常系: path_of で保存済みバイナリのパスが取得できること。
    #[test]
    fn test_path_of_returns_saved_file() {
        let (mut mgr, _dir) = tmp_manager(10);
        let v = mgr.save(b"hello plugin").unwrap();
        let path = mgr.path_of(v).expect("パスが存在するはず");
        assert!(path.exists(), "ファイルが存在するはず");
        assert_eq!(std::fs::read(path).unwrap(), b"hello plugin");
    }

    /// 正常系: max_versions を超えると古いバージョンのファイルが削除されること。
    #[test]
    fn test_purge_old_versions() {
        let (mut mgr, _dir) = tmp_manager(3);
        for i in 0..5u64 {
            mgr.save(format!("data{i}").as_bytes()).unwrap();
        }
        // 最後の3件のみ残る（v3, v4, v5）
        assert_eq!(mgr.list().len(), 3);
        assert!(mgr.path_of(1).is_none(), "v1 は削除済み");
        assert!(mgr.path_of(2).is_none(), "v2 は削除済み");
        assert!(mgr.path_of(3).is_some(), "v3 は残っているはず");
        assert!(mgr.path_of(5).is_some(), "v5 は残っているはず");
    }

    /// 正常系: current_version が save/mark_current で正しく更新されること。
    #[test]
    fn test_current_version_tracking() {
        let (mut mgr, _dir) = tmp_manager(10);
        let cases = [
            (None, None),             // 初期状態
            (Some(1u64), Some(1u64)), // save 後
            (Some(2), Some(2)),       // 2度目の save
        ];
        // 初期状態
        assert_eq!(mgr.current_version(), cases[0].1);
        // save 後
        mgr.save(b"v1").unwrap();
        assert_eq!(mgr.current_version(), cases[1].1);
        mgr.save(b"v2").unwrap();
        assert_eq!(mgr.current_version(), cases[2].1);
        // rollback
        mgr.mark_current(1);
        assert_eq!(mgr.current_version(), Some(1));
        // 存在しないバージョンへの mark_current は無視
        mgr.mark_current(99);
        assert_eq!(mgr.current_version(), Some(1));
    }

    /// 異常系: path_of で存在しないバージョンは None を返すこと。
    #[test]
    fn test_path_of_nonexistent_version() {
        let (mgr, _dir) = tmp_manager(10);
        let cases = [0u64, 1, 99];
        for v in cases {
            assert!(mgr.path_of(v).is_none(), "v{v} は存在しないはず");
        }
    }
}
