//! ホストとプラグイン間のABI契約を定義するクレート。
// abi_stable の Prefix モジュールが生成する `RobotPlugin_Ref` は規約上のアンダースコア付き名前。
#![allow(non_camel_case_types)]
//!
//! このクレートはホスト・プラグイン双方がリンクするshared interfaceであり、
//! `abi_stable` によってRustコンパイラバージョン差異を吸収する。

use abi_stable::{
    declare_root_module_statics,
    library::RootModule,
    package_version_strings,
    sabi_types::VersionStrings,
    std_types::{ROption, RResult, RSlice, RStr, RString, RVec},
    StableAbi,
};

/// ホストからプラグインへ渡す初期化コンテキスト。
#[repr(C)]
#[derive(StableAbi)]
pub struct PluginContext {
    /// ホストがプラグインインスタンスに割り当てる識別子。
    pub plugin_id: u64,
}

/// プラグインの種類。ホストがどのインターフェースを呼び出すかを決定する。
///
/// プラグインは `kind()` でこの値を返し、ホストはそれに応じたメソッドのみを呼ぶ。
#[repr(u8)]
#[derive(StableAbi, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginKind {
    /// HTTP リクエストハンドラー（Phase 4）。
    /// リクエスト時に `handle` が呼ばれる。
    Http = 0,
    // 将来追加予定:
    // Dds = 1,        // DDS制御ループ（Phase 6）
    // HttpAndDds = 2, // HTTP + DDS 両対応
}

/// ホストからプラグインへ渡す HTTP リクエスト。
#[repr(C)]
#[derive(StableAbi)]
pub struct HttpRequest {
    /// HTTP メソッド（`"GET"`, `"POST"` など）。
    pub method: RString,
    /// リクエストパス（例: `"/api/sensor/data"`）。
    pub path: RString,
    /// クエリ文字列（例: `"id=1&fmt=json"`）。クエリなしは空文字列。
    pub query: RString,
    /// リクエストボディ（バイト列）。
    pub body: RVec<u8>,
}

/// プラグインがホストへ返す HTTP レスポンス。
#[repr(C)]
#[derive(StableAbi)]
pub struct HttpResponse {
    /// HTTP ステータスコード（200, 404, 500 など）。
    pub status: u16,
    /// `Content-Type` ヘッダ値（例: `"application/json"`）。
    pub content_type: RString,
    /// レスポンスボディ（バイト列）。
    pub body: RVec<u8>,
}

/// ホストからプラグインへ渡す借用ベースの HTTP リクエスト（ゼロコピー版）。
///
/// [`HttpRequest`] の借用版。`RStr<'a>` は `&str` と同等の ABI 安定型なので
/// ホスト側でアロケーションなしに文字列を渡せる。
/// プラグインは文字列を所有する必要がなく、読み取りのみの場合に最適。
///
/// # ホストとのデータフロー
///
/// ```text
/// &str (ホスト側スタック) ──► RStr<'a> (ゼロコピー) ──► FFI ──► プラグイン
/// ```
#[repr(C)]
#[derive(StableAbi)]
pub struct HttpRequestRef<'a> {
    /// HTTP メソッド（`"GET"`, `"POST"` など）。
    pub method: RStr<'a>,
    /// リクエストパス（プレフィックス除去済み、例: `"/hello"`）。
    pub path: RStr<'a>,
    /// クエリ文字列。クエリなしは空文字列。
    pub query: RStr<'a>,
    /// リクエストボディ（バイト列）。
    pub body: RSlice<'a, u8>,
}

/// プラグインモジュールのABI定義。
///
/// Prefixモジュール方式を採用しているため、末尾への新フィールド追加は後方互換。
/// `last_prefix_field` を `shutdown` に付与しているため、Phase 6 でのDDS用フィールド追加が可能。
///
/// パスプレフィックスはホスト側がロード時に管理するため、プラグインは宣言しない。
#[repr(C)]
#[derive(StableAbi)]
#[sabi(kind(Prefix(prefix_ref = RobotPlugin_Ref)))]
#[sabi(missing_field(panic))]
pub struct RobotPlugin {
    /// プラグインの種類を返す。ホストはこれを確認してから対応するメソッドを呼ぶ。
    pub kind: extern "C" fn() -> PluginKind,

    /// プラグインの初期化。
    ///
    /// - `ctx`: ホストが渡す初期化コンテキスト。
    /// - `prev_state`: 直前の `shutdown` が返した状態バイト列。初回起動時は `RNone`。
    ///
    /// 成功時は `ROk(())` を返す。状態のデシリアライズに失敗し、かつプラグインが
    /// `on_load_error: fail` を指定している場合は `RErr(message)` を返す。
    /// ホストは `RErr` を受け取った場合、ロードを中断してエラーを返す。
    ///
    /// パスプレフィックスはホスト側が管理するため、プラグインは宣言しない。
    pub init: extern "C" fn(
        ctx: &PluginContext,
        prev_state: ROption<RSlice<'_, u8>>,
    ) -> RResult<(), RString>,

    /// HTTP リクエスト処理（`PluginKind::Http` のときのみ呼ばれる）。
    ///
    /// プラグイン内部でパニックを `catch_unwind` し、失敗時は `status=500` のレスポンスを返すこと。
    pub handle: extern "C" fn(req: &HttpRequest) -> HttpResponse,

    /// プラグインの終了処理。内部状態をバイナリ列で返す。
    ///
    /// 成功時は `ROk(bytes)` を返す。ホストは `bytes` を保持し、次の `init` の `prev_state` に渡す。
    /// シリアライズに失敗した場合は `RErr(message)` を返す。
    /// ホストはエラーをログに記録し、次回の `init` には `RNone`（新規状態）を渡す。
    #[sabi(last_prefix_field)]
    pub shutdown: extern "C" fn() -> RResult<RVec<u8>, RString>,
}

impl RootModule for RobotPlugin_Ref {
    declare_root_module_statics! {RobotPlugin_Ref}
    const BASE_NAME: &'static str = "robot_plugin";
    const NAME: &'static str = "robot_plugin";
    const VERSION_STRINGS: VersionStrings = package_version_strings!();
}

/// HTTP プラグインのボイラープレートを生成するマクロ。
///
/// # 必須パラメータ
///
/// - `name`: ログ出力に使うプラグイン名（文字列リテラル）
/// - `state`: 内部状態の型（`Default` を実装すること）
/// - `handler`: リクエストハンドラ関数（`fn(&HttpRequest, &mut State) -> HttpResponse`）
/// - `state_save`: 状態をバイト列に変換する関数（`fn(&State) -> Result<Vec<u8>, impl Display>`）
/// - `state_load`: バイト列から状態を復元する関数（`fn(&[u8]) -> Result<State, impl Display>`）
///
/// # オプションパラメータ
///
/// - `handler_ref`: ゼロコピー版ハンドラ（`fn(&HttpRequestRef<'_>, &mut State) -> HttpResponse`）
///   指定しない場合は `handler` への変換ラッパが自動生成される。
/// - `on_load_error`: `state_load` が `Err` を返したときの動作（デフォルト: `fail`）
///   - `fail`: `__init` が `RErr` を返しホストがロードを中断する（リロード時は旧バイナリで再起動）
///   - `fallback`: `Default::default()` でフォールバックしてログを出力する
///
/// # 生成されるコード
///
/// - `#[export_root_module] fn get_library()` — abi_stable エントリポイント（互換性維持用）
/// - `#[no_mangle] fn __plugin_create_ref()` — ホストがキャッシュを迂回して呼び出すエントリポイント
/// - `extern "C" fn __kind()` — `PluginKind::Http` を返す
/// - `extern "C" fn __init(ctx, prev_state) -> RResult<(), RString>` — `state_load` で状態復元
/// - `extern "C" fn __handle(req)` — `catch_unwind` ラッパ + `handler` 呼び出し
/// - `extern "C" fn __shutdown()` — `state_save` で状態保存
/// - `#[no_mangle] fn __plugin_handle_ref(req)` — ゼロコピー FFI エントリポイント
///
/// # abi_stable キャッシュの迂回
///
/// `abi_stable` の `load_from_file` はプロセスグローバルに最初のロード結果をキャッシュする。
/// 複数の `.so` を同一プロセスで動かすため、ホストは `__plugin_create_ref` を
/// `libloading` 経由で直接呼び出し、キャッシュを迂回する。
///
/// # 使用例
///
/// ```ignore
/// define_http_plugin! {
///     name: "my-plugin",
///     state: MyState,
///     handler: handle_inner,
///     state_save: save,
///     state_load: load,
///     // on_load_error 未指定 → fail（ロード失敗時にホストがロードを中断）
/// }
///
/// fn handle_inner(req: &HttpRequest, state: &mut MyState) -> HttpResponse { ... }
/// fn save(state: &MyState) -> Result<Vec<u8>, String> { ... }
/// fn load(bytes: &[u8]) -> Result<MyState, String> { ... }
/// ```
#[macro_export]
macro_rules! define_http_plugin {
    // ── `handler_ref` あり + `on_load_error` なし（デフォルト: fail）──────────
    (
        name: $name:expr,
        state: $state:ty,
        handler: $handler:ident,
        handler_ref: $handler_ref:ident,
        state_save: $save:ident,
        state_load: $load:ident $(,)?
    ) => {
        $crate::define_http_plugin! {
            @inner
            name: $name, state: $state, handler: $handler,
            state_save: $save, state_load: $load, on_load_error: fail,
        }
        $crate::define_http_plugin!(@handle_ref_impl $name, $handler_ref);
    };

    // ── `handler_ref` あり + `on_load_error` あり ────────────────────────────
    (
        name: $name:expr,
        state: $state:ty,
        handler: $handler:ident,
        handler_ref: $handler_ref:ident,
        state_save: $save:ident,
        state_load: $load:ident,
        on_load_error: $on_load_error:ident $(,)?
    ) => {
        $crate::define_http_plugin! {
            @inner
            name: $name, state: $state, handler: $handler,
            state_save: $save, state_load: $load, on_load_error: $on_load_error,
        }
        $crate::define_http_plugin!(@handle_ref_impl $name, $handler_ref);
    };

    // ── `handler_ref` なし + `on_load_error` なし（デフォルト: fail）──────────
    (
        name: $name:expr,
        state: $state:ty,
        handler: $handler:ident,
        state_save: $save:ident,
        state_load: $load:ident $(,)?
    ) => {
        $crate::define_http_plugin! {
            @inner
            name: $name, state: $state, handler: $handler,
            state_save: $save, state_load: $load, on_load_error: fail,
        }
        $crate::define_http_plugin!(@handle_ref_fallback);
    };

    // ── `handler_ref` なし + `on_load_error` あり ────────────────────────────
    (
        name: $name:expr,
        state: $state:ty,
        handler: $handler:ident,
        state_save: $save:ident,
        state_load: $load:ident,
        on_load_error: $on_load_error:ident $(,)?
    ) => {
        $crate::define_http_plugin! {
            @inner
            name: $name, state: $state, handler: $handler,
            state_save: $save, state_load: $load, on_load_error: $on_load_error,
        }
        $crate::define_http_plugin!(@handle_ref_fallback);
    };

    // ── ゼロコピー FFI エントリポイント（handler_ref あり版）─────────────────
    (@handle_ref_impl $name:expr, $handler_ref:ident) => {
        /// ゼロコピー FFI エントリポイント。
        ///
        /// `HttpRequestRef<'_>` は `RStr<'a>` を使うため、ホスト側の文字列アロケーションが不要。
        /// ホストが `libloading` 経由でこのシンボルを直接呼び出す。
        #[no_mangle]
        pub extern "C" fn __plugin_handle_ref(
            req: &$crate::HttpRequestRef<'_>,
        ) -> $crate::HttpResponse {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut state = __get_state().lock().unwrap_or_else(|e| e.into_inner());
                $handler_ref(req, &mut *state)
            }));
            match result {
                Ok(resp) => resp,
                Err(_) => {
                    eprintln!(
                        "[plugin] panic を捕捉しました（handle_ref）。500 を返します"
                    );
                    $crate::HttpResponse {
                        status: 500,
                        content_type: "text/plain".into(),
                        body: b"internal plugin error".to_vec().into(),
                    }
                }
            }
        }
    };

    // ── 変換ラッパ版 FFI エントリポイント（handler_ref なし版）──────────────
    (@handle_ref_fallback) => {
        /// 変換ラッパ版の FFI エントリポイント。
        ///
        /// `HttpRequestRef<'_>` を `HttpRequest`（所有型）へ変換してから既存ハンドラを呼ぶ。
        /// ホスト側のアロケーションは不要だが、プラグイン内部で変換コストが発生する。
        #[no_mangle]
        pub extern "C" fn __plugin_handle_ref(
            req: &$crate::HttpRequestRef<'_>,
        ) -> $crate::HttpResponse {
            let owned = $crate::HttpRequest {
                method: req.method.as_str().into(),
                path: req.path.as_str().into(),
                query: req.query.as_str().into(),
                body: req.body.as_slice().to_vec().into(),
            };
            __handle(&owned)
        }
    };

    // ── on_load_error: fail ── state_load 失敗時に RErr を返して起動を中断 ───
    (@init_load fail, $name:expr, $state:ty, $load:ident, $bytes:ident) => {
        match $load($bytes.as_slice()) {
            Ok(state) => state,
            Err(e) => {
                eprintln!(
                    "[{}] init: 状態ロード失敗: {}。起動を中止します",
                    $name, e
                );
                return abi_stable::std_types::RResult::RErr(e.to_string().into());
            }
        }
    };

    // ── on_load_error: fallback ── state_load 失敗時に Default で起動 ────────
    (@init_load fallback, $name:expr, $state:ty, $load:ident, $bytes:ident) => {
        match $load($bytes.as_slice()) {
            Ok(state) => state,
            Err(e) => {
                eprintln!(
                    "[{}] init: 状態ロード失敗: {}。デフォルト状態で起動します",
                    $name, e
                );
                <$state>::default()
            }
        }
    };

    // ── 内部実装（共通コード） ──────────────────────────────────────────────────
    (
        @inner
        name: $name:expr,
        state: $state:ty,
        handler: $handler:ident,
        state_save: $save:ident,
        state_load: $load:ident,
        on_load_error: $on_load_error:ident $(,)?
    ) => {
        static __PLUGIN_STATE: std::sync::OnceLock<std::sync::Mutex<$state>> =
            std::sync::OnceLock::new();

        fn __get_state() -> &'static std::sync::Mutex<$state> {
            __PLUGIN_STATE.get_or_init(|| std::sync::Mutex::new(<$state>::default()))
        }

        /// ホストが `libloading` 経由でキャッシュを迂回して呼び出すエントリポイント。
        ///
        /// `abi_stable` の `load_from_file` はプロセスグローバルキャッシュを使うため、
        /// 同一プロセスで複数の異なるプラグイン `.so` をロードできない。
        /// このシンボルを使うことでキャッシュを経由せず、各 `.so` から直接モジュールを取得できる。
        #[no_mangle]
        pub extern "C" fn __plugin_create_ref() -> $crate::RobotPlugin_Ref {
            use abi_stable::prefix_type::PrefixTypeTrait as _;
            $crate::RobotPlugin {
                kind: __kind,
                init: __init,
                handle: __handle,
                shutdown: __shutdown,
            }
            .leak_into_prefix()
        }

        #[abi_stable::export_root_module]
        fn get_library() -> $crate::RobotPlugin_Ref {
            use abi_stable::prefix_type::PrefixTypeTrait as _;
            $crate::RobotPlugin {
                kind: __kind,
                init: __init,
                handle: __handle,
                shutdown: __shutdown,
            }
            .leak_into_prefix()
        }

        extern "C" fn __kind() -> $crate::PluginKind {
            $crate::PluginKind::Http
        }

        extern "C" fn __init(
            _ctx: &$crate::PluginContext,
            prev_state: abi_stable::std_types::ROption<abi_stable::std_types::RSlice<'_, u8>>,
        ) -> abi_stable::std_types::RResult<(), abi_stable::std_types::RString> {
            let new_state = if let abi_stable::std_types::RSome(bytes) = prev_state {
                // `@init_load` サブアームで `on_load_error` ポリシーに応じた処理を展開する。
                // fail の場合: Err 時に `return RErr(...)` で早期リターンする。
                // fallback の場合: Err 時に `Default::default()` で継続する。
                $crate::define_http_plugin!(@init_load $on_load_error, $name, $state, $load, bytes)
            } else {
                <$state>::default()
            };
            *__get_state().lock().unwrap_or_else(|e| e.into_inner()) = new_state;
            eprintln!("[{}] init 完了", $name);
            abi_stable::std_types::RResult::ROk(())
        }

        extern "C" fn __handle(req: &$crate::HttpRequest) -> $crate::HttpResponse {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                // catch_unwind 内でパニックが起きると MutexGuard の drop で mutex が
                // poisoned になるため、unwrap_or_else で poisoned mutex を回復する。
                let mut state = __get_state().lock().unwrap_or_else(|e| e.into_inner());
                $handler(req, &mut *state)
            }));
            match result {
                Ok(resp) => resp,
                Err(_) => {
                    eprintln!("[{}] panic を捕捉しました。500 を返します", $name);
                    $crate::HttpResponse {
                        status: 500,
                        content_type: "text/plain".into(),
                        body: b"internal plugin error".to_vec().into(),
                    }
                }
            }
        }

        extern "C" fn __shutdown() -> abi_stable::std_types::RResult<
            abi_stable::std_types::RVec<u8>,
            abi_stable::std_types::RString,
        > {
            let state = __get_state().lock().unwrap_or_else(|e| e.into_inner());
            eprintln!("[{}] shutdown", $name);
            match $save(&*state) {
                Ok(bytes) => abi_stable::std_types::RResult::ROk(bytes.into()),
                Err(e) => {
                    eprintln!("[{}] shutdown: 状態シリアライズ失敗: {}", $name, e);
                    abi_stable::std_types::RResult::RErr(e.to_string().into())
                }
            }
        }
    };
}
