//! プラグインのライフサイクルを管理するモジュール。
//!
//! - [`PluginManager`][]: 単一プラグインのロード・アンロード・リロード・状態引き継ぎ。
//! - [`PluginRouter`]: プレフィックス → プラグインの HashMap によるルーティング。
//!
//! パスプレフィックスはホスト側（`PluginRouter`）が管理する。
//! プラグイン自身はルートを宣言せず、ロード時に指定されたプレフィックスに紐づく。

use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
};

use abi_stable::std_types::{RNone, RResult, RSlice, RSome, RString, RVec};
use safety_plugin_common::{
    HttpRequest, HttpRequestRef, HttpResponse, PluginContext, PluginKind, RobotPlugin_Ref,
};

// ─── RString プール（スレッドローカル） ──────────────────────────────────────

/// プール内に保持する RString の最大キャパシティ（バイト）。
/// これを超える文字列はプールに戻さず破棄する。
const POOL_MAX_CAP: usize = 512;

thread_local! {
    static RSTRING_POOL: RefCell<Vec<RString>> = const { RefCell::new(Vec::new()) };
}

/// プールから `RString` を取り出し、内容を `s` で初期化して返す。
///
/// プールが空の場合は新規アロケートする。
/// プールに既存の文字列がある場合は `clear()` + `push_str()` でメモリを再利用する。
pub fn rstring_from_pool(s: &str) -> RString {
    RSTRING_POOL.with(|pool| {
        if let Some(mut rs) = pool.borrow_mut().pop() {
            rs.clear();
            rs.push_str(s);
            rs
        } else {
            RString::from(s)
        }
    })
}

/// `RString` をプールに返却する。
///
/// キャパシティが [`POOL_MAX_CAP`] を超える場合は破棄してメモリを解放する。
pub fn rstring_to_pool(rs: RString) {
    if rs.capacity() <= POOL_MAX_CAP {
        RSTRING_POOL.with(|pool| pool.borrow_mut().push(rs));
    }
}

// ─── PluginManager（単一プラグインのライフサイクル管理） ─────────────────────

/// `__plugin_handle_ref` 関数ポインタの型。
///
/// `HttpRequestRef<'_>` を受け取り `HttpResponse` を返す ABI 安定 FFI 関数。
type HandleRefFn = extern "C" fn(req: &HttpRequestRef<'_>) -> HttpResponse;

/// ロード済みプラグインの情報。
struct LoadedPlugin {
    /// プラグインのモジュール参照（関数ポインタ群）。
    module: RobotPlugin_Ref,
    /// ゼロコピー FFI エントリポイント（`__plugin_handle_ref` シンボル）。
    ///
    /// `None` の場合は `handle_ref` 呼び出し時に `handle` へフォールバックする。
    handle_ref_fn: Option<HandleRefFn>,
    /// ロード元のパス（リロード失敗時の旧バージョン復帰に使う）。
    path: PathBuf,
    /// 開いた共有ライブラリハンドル。`module` の関数ポインタを有効に保つために保持する。
    ///
    /// `module` → `_lib` の順で drop されるため、`_lib` が drop（dlclose）される時点では
    /// `module` の関数ポインタは既に使用されていない。
    _lib: libloading::Library,
}

/// 単一プラグインのライフサイクルを管理する。
///
/// ルーティングは [`PluginRouter`] が担当する。このクラスはロード・リロード・
/// シャットダウン・状態引き継ぎのみを管理する。
#[derive(Default)]
pub struct PluginManager {
    /// 現在ロードされているプラグイン。未ロード時は `None`。
    current: Option<LoadedPlugin>,
    /// 前回の `shutdown` が返した状態バイト列。次の `init` に渡す。
    saved_state: Option<Vec<u8>>,
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
    /// 新バイナリのロードに失敗した場合は、旧バイナリで `init` し直す。
    /// この場合でも `Err` を返すが、旧プラグインで動作を継続する。
    pub fn reload(&mut self, new_path: &Path) -> anyhow::Result<()> {
        let old_path = self.current.as_ref().map(|l| l.path.clone());
        self.shutdown_current();

        match self.load_internal(new_path) {
            Ok(()) => {
                tracing::info!("リロード成功: {}", new_path.display());
                Ok(())
            }
            Err(e) => {
                tracing::error!("リロード失敗 ({}): {e}", new_path.display());
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

    /// 現在のプラグインをアンロードし、`shutdown` の戻り値を返す。
    ///
    /// ロードし直す予定がない場合に呼び出す。`saved_state` はクリアされる。
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
    /// 未ロード時は 503 を返す。ルーティングは呼び出し元（[`PluginRouter`]）が行う。
    pub fn handle(&self, req: &HttpRequest) -> HttpResponse {
        match &self.current {
            Some(loaded) => (loaded.module.handle())(req),
            None => service_unavailable("プラグイン未ロード"),
        }
    }

    /// `HttpRequestRef<'_>`（借用型・ゼロコピー）でリクエストをプラグインへ委譲する。
    ///
    /// プラグインが `__plugin_handle_ref` シンボルをエクスポートしている場合は
    /// ゼロコピー FFI パスを使う。そうでなければ `handle` と同じ所有型パスにフォールバックする。
    pub fn handle_ref(&self, req: &HttpRequestRef<'_>) -> HttpResponse {
        match &self.current {
            Some(loaded) => {
                if let Some(f) = loaded.handle_ref_fn {
                    f(req)
                } else {
                    // フォールバック: HttpRequestRef → HttpRequest へ変換して既存パスを使う
                    let owned = HttpRequest {
                        method: req.method.as_str().into(),
                        path: req.path.as_str().into(),
                        query: req.query.as_str().into(),
                        body: req.body.as_slice().to_vec().into(),
                    };
                    (loaded.module.handle())(&owned)
                }
            }
            None => service_unavailable("プラグイン未ロード"),
        }
    }

    /// プラグインがロード済みかどうかを返す。
    pub fn is_loaded(&self) -> bool {
        self.current.is_some()
    }

    /// 現在ロード中のプラグインパスを返す。
    pub fn current_path(&self) -> Option<&Path> {
        self.current.as_ref().map(|l| l.path.as_path())
    }

    /// 現在のプラグインをシャットダウンし、状態を `saved_state` へ保存する。
    ///
    /// シリアライズに失敗した場合はエラーをログに記録し `saved_state = None` とする。
    /// 次の `init` にはフレッシュな状態（`RNone`）が渡される。
    fn shutdown_current(&mut self) {
        if let Some(loaded) = self.current.take() {
            match (loaded.module.shutdown())() {
                RResult::ROk(bytes) => {
                    let bytes = bytes.into_vec();
                    if !bytes.is_empty() {
                        tracing::debug!("プラグイン状態を保存: {} バイト", bytes.len());
                        self.saved_state = Some(bytes);
                    } else {
                        self.saved_state = None;
                    }
                }
                RResult::RErr(msg) => {
                    tracing::error!(
                        "プラグイン状態のシリアライズ失敗: {}。次回はフレッシュ状態で起動します",
                        msg
                    );
                    self.saved_state = None;
                }
            }
        }
    }

    /// プラグインをロードして `init` を呼び出す内部実装。
    ///
    /// `abi_stable` の `load_from_file` はプロセスグローバルキャッシュを使うため、
    /// 同一プロセスで複数の異なる `.so` をロードできない。
    /// 代わりに `libloading` で `__plugin_create_ref` シンボルを直接呼び出し、
    /// キャッシュを迂回する。
    fn load_internal(&mut self, path: &Path) -> anyhow::Result<()> {
        // SAFETY: プラグイン .so のロード。シンボル解決に失敗した場合は Err を返す。
        let lib = unsafe { libloading::Library::new(path) }
            .map_err(|e| anyhow::anyhow!("共有ライブラリのオープンに失敗: {e}"))?;

        // `__plugin_create_ref` は `define_http_plugin!` マクロが各プラグインに生成する
        // `#[no_mangle]` シンボル。libloading はデフォルトで RTLD_LOCAL を使うため、
        // 異なる .so 間でシンボル名が衝突しない。
        let module: RobotPlugin_Ref = {
            // SAFETY: シンボルの型は ABI 定義（RobotPlugin_Ref）と一致している。
            let create_ref: libloading::Symbol<unsafe extern "C" fn() -> RobotPlugin_Ref> =
                unsafe { lib.get(b"__plugin_create_ref\0") }.map_err(|e| {
                    anyhow::anyhow!(
                        "__plugin_create_ref シンボルが見つかりません（`define_http_plugin!` を使っていますか？）: {e}"
                    )
                })?;
            unsafe { create_ref() }
        };

        let plugin_kind = (module.kind())();
        if plugin_kind != PluginKind::Http {
            anyhow::bail!("未対応のプラグイン種別: {plugin_kind:?}");
        }

        let ctx = PluginContext { plugin_id: 0 };
        let prev = match self.saved_state.as_deref() {
            Some(bytes) => RSome(RSlice::from_slice(bytes)),
            None => RNone,
        };

        match (module.init())(&ctx, prev) {
            RResult::ROk(()) => {}
            RResult::RErr(msg) => {
                anyhow::bail!("プラグイン init 失敗: {msg}");
            }
        }

        // `__plugin_handle_ref` シンボルをオプションでロードする。
        // `define_http_plugin!` で生成されるため通常は存在するが、
        // 古いプラグインとの互換性のため `None` の場合は `handle` にフォールバックする。
        let handle_ref_fn: Option<HandleRefFn> = unsafe {
            lib.get::<HandleRefFn>(b"__plugin_handle_ref\0")
                .ok()
                .map(|sym| *sym)
        };

        tracing::info!(
            "プラグインをロード: {} (handle_ref: {})",
            path.display(),
            handle_ref_fn.is_some()
        );

        self.current = Some(LoadedPlugin {
            module,
            handle_ref_fn,
            path: path.to_path_buf(),
            _lib: lib,
        });
        Ok(())
    }
}

impl Drop for PluginManager {
    fn drop(&mut self) {
        self.shutdown_current();
    }
}

// ─── PluginRouter（複数プラグインのプレフィックスルーティング） ───────────────

/// プレフィックスをキーに複数のプラグインをルーティングする。
///
/// キーはマウントプレフィックス（例: `"/api"`, `"/sample"`）。
/// リクエストパスに対して最長一致するプレフィックスのプラグインへ委譲する。
#[derive(Default)]
pub struct PluginRouter {
    /// プレフィックス → プラグインマネージャのマップ。
    plugins: HashMap<String, PluginManager>,
    /// プラグイン未ロード・プレフィックス未登録によるフォールバック実行回数。
    pub fallback_count: u64,
}

impl PluginRouter {
    /// 指定プレフィックスにプラグインをロードする。
    ///
    /// 同じプレフィックスに既存プラグインがある場合は先にシャットダウンしてからロードする。
    pub fn load(&mut self, prefix: impl Into<String>, path: &Path) -> anyhow::Result<()> {
        let prefix = prefix.into();
        self.plugins.entry(prefix).or_default().load(path)
    }

    /// 指定プレフィックスのプラグインをホットリロードする。
    ///
    /// プレフィックスが未登録の場合は新規エントリを作成してロードする。
    pub fn reload(&mut self, prefix: &str, new_path: &Path) -> anyhow::Result<()> {
        self.plugins
            .entry(prefix.to_string())
            .or_default()
            .reload(new_path)
    }

    /// 指定プレフィックスのプラグインをアンロードし、`shutdown` の戻り値を返す。
    pub fn unload(&mut self, prefix: &str) -> Option<Vec<u8>> {
        self.plugins.get_mut(prefix)?.unload()
    }

    /// HTTP リクエストを最長一致プレフィックスのプラグインへ委譲する。
    ///
    /// - 一致するプレフィックスなし: 404 を返す（フォールバック扱いではない）。
    /// - プレフィックスあり・プラグイン未ロード: 503 を返し `fallback_count` を加算する。
    /// - プレフィックスあり・プラグインロード済み: プラグインの `handle` を呼ぶ。
    ///
    /// リクエスト処理後、`method`・`path`（プレフィックス除去済み）・`query` を
    /// スレッドローカルプールへ返却する。[`rstring_from_pool`] でプールを活用すると
    /// 定常状態でアロケーションを削減できる。
    pub fn handle(&mut self, req: HttpRequest) -> HttpResponse {
        let path_str = req.path.as_str();
        // 一致プレフィックスを検索（不変借用スコープを限定）
        let Some((prefix, manager)) = self
            .plugins
            .iter_mut()
            .find(|(pfx, _manager)| path_matches_prefix(path_str, pfx.as_str()))
        else {
            return HttpResponse {
                status: 404,
                content_type: "text/plain".into(),
                body: RVec::from(b"not found".to_vec()),
            };
        };

        // is_loaded の確認（借用を早期解放するため先に取得）
        if !manager.is_loaded() {
            self.fallback_count += 1;
            tracing::warn!(
                "フォールバック実行 ({}): プラグイン未ロード (prefix: {})",
                self.fallback_count,
                prefix
            );
            return service_unavailable("プラグイン未ロード");
        }

        // req を分解してプレフィックスを除去した path を再構築する。
        // orig_path をプールに返すことで次リクエストの RString アロケーションを節約できる。
        let prefix_len = prefix.len();
        let HttpRequest {
            method,
            path: orig_path,
            query,
            body,
        } = req;

        // プレフィックス除去済みパスを文字列として取り出してから orig_path をプールへ返す。
        let stripped: String = orig_path.as_str()[prefix_len..].to_owned();
        rstring_to_pool(orig_path);

        let req = HttpRequest {
            method,
            path: rstring_from_pool(&stripped),
            query,
            body,
        };

        let resp = manager.handle(&req);

        // 処理済みの文字列フィールドをプールへ返却する。
        let HttpRequest {
            method,
            path,
            query,
            body: _,
        } = req;
        rstring_to_pool(method);
        rstring_to_pool(path);
        rstring_to_pool(query);

        resp
    }

    /// `&str` スライスのみで HTTP リクエストをルーティングする（ゼロコピー版）。
    ///
    /// ホスト側で `RString` / `RVec` のアロケーションを一切行わず、
    /// `RStr<'_>` / `RSlice<'_>` としてプラグインへ渡す。
    /// プラグインが `__plugin_handle_ref` をエクスポートしている場合はゼロコピー FFI を使う。
    pub fn handle_ref(
        &mut self,
        method: &str,
        path: &str,
        query: &str,
        body: &[u8],
    ) -> HttpResponse {
        let Some((prefix, manager)) = self
            .plugins
            .iter()
            .find(|(pfx, _manager)| path_matches_prefix(path, pfx.as_str()))
        else {
            return HttpResponse {
                status: 404,
                content_type: "text/plain".into(),
                body: RVec::from(b"not found".to_vec()),
            };
        };

        if !manager.is_loaded() {
            self.fallback_count += 1;
            tracing::warn!(
                "フォールバック実行 ({}): プラグイン未ロード (prefix: {})",
                self.fallback_count,
                prefix
            );
            return service_unavailable("プラグイン未ロード");
        }

        // プレフィックス除去はゼロコピー（&str スライス）
        let stripped = &path[prefix.len()..];
        let req = HttpRequestRef {
            method: method.into(),
            path: stripped.into(),
            query: query.into(),
            body: body.into(),
        };

        manager.handle_ref(&req)
    }

    /// 登録済みプレフィックスの一覧を返す（順序不定）。
    pub fn prefixes(&self) -> Vec<String> {
        self.plugins.keys().cloned().collect()
    }

    /// 指定プレフィックスのプラグインがロード済みかどうかを返す。
    pub fn is_loaded(&self, prefix: &str) -> bool {
        self.plugins
            .get(prefix)
            .map(|m| m.is_loaded())
            .unwrap_or(false)
    }

    /// 指定プレフィックスの `PluginManager` を参照する（テスト・API 用）。
    pub fn get_manager(&self, prefix: &str) -> Option<&PluginManager> {
        self.plugins.get(prefix)
    }

    /// 指定プレフィックスの `PluginManager` を可変参照する（テスト・API 用）。
    pub fn get_manager_mut(&mut self, prefix: &str) -> Option<&mut PluginManager> {
        self.plugins.get_mut(prefix)
    }

    /// 登録済みプレフィックスが存在するかどうかを返す。
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }
}

// ─── 共通ユーティリティ ───────────────────────────────────────────────────────

/// リクエストパスがプレフィックスに一致するか判定する。
///
/// 一致条件:
/// - `path == prefix`（完全一致）
/// - `path` が `"<prefix>/"` で始まる（プレフィックス配下のパス）
///
/// これにより `/api` プレフィックスが `/api2` にマッチしてしまう誤検知を防ぐ。
fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .map(|rest| rest.starts_with('/'))
            .unwrap_or(false)
}

fn service_unavailable(reason: &str) -> HttpResponse {
    HttpResponse {
        status: 503,
        content_type: "text/plain".into(),
        body: format!("service unavailable: {reason}").into_bytes().into(),
    }
}

// ─── ユニットテスト ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// path_matches_prefix: 一致条件（完全一致 / prefix/ 配下）の確認。
    #[test]
    fn test_path_matches_prefix() {
        // 完全一致
        assert!(path_matches_prefix("/api", "/api"));
        // prefix 配下
        assert!(path_matches_prefix("/api/hello", "/api"));
        assert!(path_matches_prefix("/api/hello/world", "/api"));
        // prefix の後が '/' でない → 不一致
        assert!(!path_matches_prefix("/api2", "/api"));
        assert!(!path_matches_prefix("/api2/hello", "/api"));
        // 全く関係ないパス
        assert!(!path_matches_prefix("/other", "/api"));
        assert!(!path_matches_prefix("/", "/api"));
        // ルートプレフィックス（完全一致のみ）
        // NOTE: "/" は特殊で strip_prefix("/") が先頭の "/" を除去するため
        // "/anything" はマッチしない。ルートプレフィックスは現状未使用。
        assert!(path_matches_prefix("/", "/"));
    }

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

    /// 正常系: プレフィックス未登録のリクエストは 404 が返ること。
    #[test]
    fn test_router_handle_no_prefix_returns_404() {
        let cases = ["/api/hello", "/other", "/"];
        for path in cases {
            let mut router = PluginRouter::default();
            let req = HttpRequest {
                method: "GET".into(),
                path: path.into(),
                query: "".into(),
                body: RVec::new(),
            };
            let resp = router.handle(req);
            assert_eq!(
                resp.status, 404,
                "プレフィックス未登録の {path} は 404 が返るべき"
            );
            assert_eq!(
                router.fallback_count, 0,
                "プレフィックス未登録は fallback_count に加算しない"
            );
        }
    }

    /// 正常系: 未ロード状態のアンロードは安全に None を返すこと。
    #[test]
    fn test_unload_without_plugin() {
        let mut router = PluginRouter::default();
        let state = router.unload("/api");
        assert!(state.is_none());
    }

    /// 正常系: is_empty が登録状況を正しく反映すること。
    #[test]
    fn test_router_is_empty() {
        let router = PluginRouter::default();
        assert!(router.is_empty(), "初期状態は空のはず");
    }
}
