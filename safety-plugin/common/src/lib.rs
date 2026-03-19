//! ホストとプラグイン間のABI契約を定義するクレート。
// abi_stable の Prefix モジュールが生成する `RobotPlugin_Ref` は規約上のアンダースコア付き名前。
#![allow(non_camel_case_types)]
//!
//! このクレートはホスト・プラグイン双方がリンクするshared interfaceであり、
//! `abi_stable` によってRustコンパイラバージョン差異を吸収する。

use abi_stable::{
    declare_root_module_statics, library::RootModule, package_version_strings,
    sabi_types::VersionStrings,
    std_types::{ROption, RSlice, RString, RVec},
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
    /// `init` が `RouteDescriptor` を返し、リクエスト時に `handle` が呼ばれる。
    Http = 0,
    // 将来追加予定:
    // Dds = 1,        // DDS制御ループ（Phase 6）
    // HttpAndDds = 2, // HTTP + DDS 両対応
}

/// プラグインが `init` 時に宣言する担当パス記述子（`PluginKind::Http` 用）。
#[repr(C)]
#[derive(StableAbi)]
pub struct RouteDescriptor {
    /// 担当するパスプレフィックス（例: `"/api/sensor"`）。
    /// ホストはこのプレフィックスで始まるリクエストをプラグインへ委譲する。
    pub path_prefix: RString,
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

/// プラグインモジュールのABI定義。
///
/// Prefixモジュール方式を採用しているため、末尾への新フィールド追加は後方互換。
/// `last_prefix_field` を `shutdown` に付与しているため、Phase 6 でのDDS用フィールド追加が可能。
#[repr(C)]
#[derive(StableAbi)]
#[sabi(kind(Prefix(prefix_ref = RobotPlugin_Ref)))]
#[sabi(missing_field(panic))]
pub struct RobotPlugin {
    /// プラグインの種類を返す。ホストはこれを確認してから対応するメソッドを呼ぶ。
    pub kind: extern "C" fn() -> PluginKind,

    /// プラグインの初期化。担当ルート一覧を返す（`PluginKind::Http` 用）。
    ///
    /// - `ctx`: ホストが渡す初期化コンテキスト。
    /// - `prev_state`: 直前の `shutdown` が返した状態バイト列。初回起動時は `RNone`。
    pub init: extern "C" fn(
        ctx: &PluginContext,
        prev_state: ROption<RSlice<'_, u8>>,
    ) -> RVec<RouteDescriptor>,

    /// HTTP リクエスト処理（`PluginKind::Http` のときのみ呼ばれる）。
    ///
    /// プラグイン内部でパニックを `catch_unwind` し、失敗時は `status=500` のレスポンスを返すこと。
    pub handle: extern "C" fn(req: &HttpRequest) -> HttpResponse,

    /// プラグインの終了処理。内部状態をバイナリ列で返す。
    ///
    /// 返したバイト列はホストが保持し、次の `init` の `prev_state` に渡される。
    /// 状態がない場合は空の `RVec` を返す。
    #[sabi(last_prefix_field)]
    pub shutdown: extern "C" fn() -> RVec<u8>,
}

impl RootModule for RobotPlugin_Ref {
    declare_root_module_statics! {RobotPlugin_Ref}
    const BASE_NAME: &'static str = "robot_plugin";
    const NAME: &'static str = "robot_plugin";
    const VERSION_STRINGS: VersionStrings = package_version_strings!();
}
