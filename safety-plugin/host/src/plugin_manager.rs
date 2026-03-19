//! プラグインのライフサイクルを管理するモジュール。
//!
//! - プラグインのロード・アンロード・リロード
//! - HTTP リクエストのプラグインへの委譲
//! - パスプレフィックスに基づくルーティング
//! - フォールバックレスポンスの生成
//! - ホットリロード時の状態引き継ぎ

use std::path::{Path, PathBuf};

use abi_stable::{
    library::RootModule,
    std_types::{RNone, RSlice, RSome, RVec},
};
use safety_plugin_common::{HttpRequest, HttpResponse, PluginContext, PluginKind, RobotPlugin_Ref};

/// ロード済みプラグインの情報。
struct LoadedPlugin {
    /// abi_stable 管理のプラグインモジュール参照。
    module: RobotPlugin_Ref,
    /// プラグインが宣言した担当パスプレフィックス一覧。
    routes: Vec<String>,
    /// ロード元のパス（リロード失敗時の旧バージョン復帰に使う）。
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
        self.shutdown_current();
        self.load_internal(path)
    }

    /// 動作中に新しいプラグインへ切り替える（ホットリロード）。
    ///
    /// 新バイナリのロードに失敗した場合は、旧バイナリの saved_state で `init` し直す。
    /// この場合でも `Err` を返すが、マネージャは旧プラグインで動作を継続する。
    pub fn reload(&mut self, new_path: &Path) -> anyhow::Result<()> {
        // 旧プラグインのパスを保持してからシャットダウン（状態を saved_state へ保存）
        let old_path = self.current.as_ref().map(|l| l.path.clone());
        self.shutdown_current();

        match self.load_internal(new_path) {
            Ok(()) => {
                tracing::info!("リロード成功: {}", new_path.display());
                Ok(())
            }
            Err(e) => {
                tracing::error!("リロード失敗 ({}): {e}", new_path.display());
                // 旧バイナリで再起動を試みる（saved_state は保持されているので状態も復元される）
                if let Some(old) = old_path {
                    tracing::info!("旧バージョンで再起動: {}", old.display());
                    if let Err(e2) = self.load_internal(&old) {
                        tracing::error!("旧バージョンの再起動にも失敗: {e2}");
                    }
                }
                Err(e)
            }
        }
    }

    /// 現在のプラグインをアンロードし、shutdown の戻り値を返す。
    ///
    /// ロードし直す予定がない場合に呼び出す。saved_state はクリアされる。
    pub fn unload(&mut self) -> Option<Vec<u8>> {
        self.shutdown_current();
        self.saved_state.take()
    }

    /// 前回の `shutdown` が返した状態バイト列を参照する（テスト・デバッグ用）。
    pub fn saved_state(&self) -> Option<&[u8]> {
        self.saved_state.as_deref()
    }

    /// HTTP リクエストをプラグインへ委譲する。
    ///
    /// - プラグイン未ロード: 503 レスポンスを返し `fallback_count` を加算する。
    /// - パスが担当ルートに未マッチ: 404 レスポンスを返す（フォールバック扱いではない）。
    /// - マッチ: プラグインの `handle` を呼ぶ（パニックはプラグイン側で catch_unwind 済み）。
    pub fn handle(&mut self, req: &HttpRequest) -> HttpResponse {
        let Some(loaded) = &self.current else {
            self.fallback_count += 1;
            tracing::warn!(
                "フォールバック実行 ({}): プラグイン未ロード",
                self.fallback_count
            );
            return Self::service_unavailable("プラグイン未ロード");
        };

        // パスプレフィックスマッチ（ホスト側ルーティング）
        let path = req.path.as_str();
        let matched = loaded.routes.iter().any(|r| path.starts_with(r.as_str()));
        if !matched {
            return HttpResponse {
                status: 404,
                content_type: "text/plain".into(),
                body: RVec::from(b"not found".to_vec()),
            };
        }

        // プラグインへ委譲（パニックはプラグイン側の catch_unwind で処理済み）
        (loaded.module.handle())(req)
    }

    /// 現在のプラグインをシャットダウンし、状態を `saved_state` へ保存する。
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

        // kind() でプラグイン種類を確認
        let plugin_kind = (module.kind())();
        if plugin_kind != PluginKind::Http {
            anyhow::bail!("未対応のプラグイン種別: {plugin_kind:?}");
        }

        let ctx = PluginContext { plugin_id: 0 };
        let prev = match self.saved_state.as_deref() {
            Some(bytes) => RSome(RSlice::from_slice(bytes)),
            None => RNone,
        };

        let route_descs = (module.init())(&ctx, prev);
        let routes: Vec<String> = route_descs
            .iter()
            .map(|d| d.path_prefix.to_string())
            .collect();

        tracing::info!(
            "プラグインをロード: {} （ルート {}件: {:?}）",
            path.display(),
            routes.len(),
            routes,
        );

        self.current = Some(LoadedPlugin {
            module,
            routes,
            path: path.to_path_buf(),
        });
        Ok(())
    }

    /// プラグインが利用不可の場合のフォールバックレスポンス（503）。
    fn service_unavailable(reason: &str) -> HttpResponse {
        HttpResponse {
            status: 503,
            content_type: "text/plain".into(),
            body: format!("service unavailable: {reason}").into_bytes().into(),
        }
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
        // Drop でシャットダウンして状態をプラグインが解放できるようにする
        self.shutdown_current();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use abi_stable::std_types::RVec;

    /// 値域確認: HttpResponse のステータスコードが正しく設定できること。
    #[test]
    fn test_http_response_status_codes() {
        let cases = [
            (200u16, "ok"),
            (404, "not found"),
            (500, "error"),
            (503, "unavailable"),
        ];
        for (status, body_str) in cases {
            let resp = HttpResponse {
                status,
                content_type: "text/plain".into(),
                body: RVec::from(body_str.as_bytes().to_vec()),
            };
            assert_eq!(resp.status, status);
        }
    }

    /// 正常系: プラグイン未ロード時は 503 が返り、fallback_count が加算される。
    #[test]
    fn test_handle_without_plugin() {
        let cases = [1usize, 3, 5];
        for n in cases {
            let mut mgr = PluginManager::default();
            let req = HttpRequest {
                method: "GET".into(),
                path: "/api/hello".into(),
                query: "".into(),
                body: RVec::new(),
            };
            for _ in 0..n {
                let resp = mgr.handle(&req);
                assert_eq!(resp.status, 503, "未ロード時は 503 が返るべき");
            }
            assert_eq!(mgr.fallback_count, n as u64, "fallback_count が一致しない");
        }
    }

    /// 正常系: 未ロード状態のアンロードは安全に何もしない。
    #[test]
    fn test_unload_without_plugin() {
        let mut mgr = PluginManager::default();
        let state = mgr.unload();
        assert!(state.is_none());
    }
}
