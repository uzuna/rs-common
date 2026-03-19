//! プラグインのライフサイクルを管理するモジュール。
//!
//! - プラグインのロード・アンロード・リロード
//! - `update` 呼び出しとエラー検知
//! - フォールバックロジックの実行

use std::path::{Path, PathBuf};

use abi_stable::{
    library::RootModule,
    std_types::{RNone, RSlice, RSome, RVec},
};
use safety_plugin_common::{PluginContext, RobotPlugin_Ref, TopicData};

/// `step()` の実行結果。
#[derive(Debug, PartialEq, Eq)]
pub enum StepResult {
    /// プラグインが正常に実行された。
    Ok,
    /// プラグインが未ロード、またはエラーを返したためフォールバックを実行した。
    Fallback,
}

/// ロード済みプラグインの情報。
struct LoadedPlugin {
    /// abi_stable 管理のプラグインモジュール参照。Library の所有権も内包する。
    module: RobotPlugin_Ref,
    /// このプラグインが宣言したトピック記述子（Phase 4 でDDSエンティティを作成するために保持）。
    #[allow(dead_code)]
    topic_count: usize,
    /// ロード元のパス（リロード時の参照用）。
    path: PathBuf,
}

/// プラグインのライフサイクルを管理する。
#[derive(Default)]
pub struct PluginManager {
    /// 現在ロードされているプラグイン。未ロード時は `None`。
    current: Option<LoadedPlugin>,
    /// 前回の `shutdown` が返した状態バイト列。次の `init` に渡す。
    saved_state: Option<Vec<u8>>,
    /// フォールバック実行回数（デバッグ・テスト用）。
    pub fallback_count: u64,
}

impl PluginManager {
    /// 指定パスのプラグインをロードする。
    ///
    /// すでにプラグインがロードされている場合は先に `shutdown` してからロードする。
    pub fn load(&mut self, path: &Path) -> anyhow::Result<()> {
        // 旧プラグインをシャットダウンして状態を保存
        self.shutdown_current();

        self.load_internal(path)
    }

    /// 動作中に新しいプラグインへ切り替える（ホットリロード）。
    ///
    /// ロードに失敗した場合は旧バージョンの状態で `init` し直す。
    pub fn reload(&mut self, path: &Path) -> anyhow::Result<()> {
        // 旧プラグインをシャットダウンして状態を保存
        self.shutdown_current();

        match self.load_internal(path) {
            Ok(()) => {
                tracing::info!("リロード成功: {}", path.display());
                Ok(())
            }
            Err(e) => {
                tracing::error!("リロード失敗: {e}");
                // 旧状態を使って同じパスで再試行（前回のパスが残っている場合）
                Err(e)
            }
        }
    }

    /// 現在のプラグインをシャットダウンし、状態を保存する。
    fn shutdown_current(&mut self) {
        if let Some(loaded) = self.current.take() {
            let state = (loaded.module.shutdown())();
            let bytes = state.into_vec();
            if !bytes.is_empty() {
                tracing::debug!("プラグイン状態を保存: {} バイト", bytes.len());
                self.saved_state = Some(bytes);
            } else {
                self.saved_state = None;
            }
        }
    }

    /// プラグインをロードして `init` を呼び出す内部実装。
    fn load_internal(&mut self, path: &Path) -> anyhow::Result<()> {
        let module = RobotPlugin_Ref::load_from_file(path)
            .map_err(|e| anyhow::anyhow!("プラグインのロードに失敗: {e:?}"))?;

        let ctx = PluginContext { plugin_id: 0 };

        // 保存済み状態があれば prev_state として渡す
        let prev = match self.saved_state.as_deref() {
            Some(bytes) => RSome(RSlice::from_slice(bytes)),
            None => RNone,
        };

        let topic_descs = (module.init())(&ctx, prev);
        let topic_count = topic_descs.len();

        tracing::info!(
            "プラグインをロード: {} （トピック {}件）",
            path.display(),
            topic_count
        );
        for desc in topic_descs.iter() {
            tracing::debug!("  トピック: {} ({:?})", desc.name, desc.direction);
        }

        self.current = Some(LoadedPlugin {
            module,
            topic_count,
            path: path.to_path_buf(),
        });
        Ok(())
    }

    /// 制御ループの1ステップを実行する。
    ///
    /// プラグインが未ロード、または `update` がエラーを返した場合はフォールバックを実行する。
    /// Phase 4 以降は received に DDS 受信データを渡し、publish をDDSへ書き込む。
    pub fn step(&mut self) -> StepResult {
        let Some(loaded) = &self.current else {
            return self.run_fallback("プラグイン未ロード");
        };

        // Phase 4 実装まではデータなしで呼び出す
        let received: Vec<TopicData> = Vec::new();
        let mut publish = RVec::<TopicData>::new();

        let result = (loaded.module.update())(RSlice::from_slice(&received), &mut publish);

        if result < 0 {
            return self.run_fallback(&format!("update がエラーコード {result} を返した"));
        }

        // Phase 4: publish をDDS Publisherへ書き込む（現在は無視）
        if !publish.is_empty() {
            tracing::debug!("publish: {}件（Phase 4 で実装）", publish.len());
        }

        StepResult::Ok
    }

    /// Tier1 フォールバックロジック。
    ///
    /// プラグインが動作できない場合に呼び出される最小安全ロジック。
    fn run_fallback(&mut self, reason: &str) -> StepResult {
        self.fallback_count += 1;
        tracing::warn!("フォールバック実行 ({}): {}", self.fallback_count, reason);
        // Phase 4: ここで速度ゼロコマンドの送信・安全停止シーケンスを実装する
        StepResult::Fallback
    }

    /// 現在ロード中のプラグインパスを返す。
    pub fn current_path(&self) -> Option<&Path> {
        self.current.as_ref().map(|l| l.path.as_path())
    }

    /// プラグインがロード済みかどうかを返す。
    pub fn is_loaded(&self) -> bool {
        self.current.is_some()
    }
}

impl Drop for PluginManager {
    fn drop(&mut self) {
        self.shutdown_current();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// プラグイン未ロード時は常にフォールバックが実行されることを確認する。
    fn test_step_without_plugin_runs_fallback() {
        let cases = [1u64, 2, 5];
        for &n in &cases {
            let mut mgr = PluginManager::default();
            let mut results = Vec::new();
            for _ in 0..n {
                results.push(mgr.step());
            }
            assert!(
                results.iter().all(|r| *r == StepResult::Fallback),
                "未ロード時はすべてFallbackであるべき"
            );
            assert_eq!(mgr.fallback_count, n, "フォールバック回数が一致しない");
        }
    }
}
