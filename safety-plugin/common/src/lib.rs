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

/// プラグインが `init` 時に宣言するトピック記述子。
/// ホストはこの一覧にもとづいてDDSエンティティを作成する。
#[repr(C)]
#[derive(StableAbi)]
pub struct TopicDescriptor {
    /// トピック名（例: `"robot/cmd_vel"`）。
    pub name: RString,
    /// トピックのデータ方向。
    pub direction: TopicDirection,
    /// QoSプリセット。
    pub qos: QosPreset,
}

/// トピックのデータ方向。
#[repr(u8)]
#[derive(StableAbi, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopicDirection {
    /// ホストが Publisher を作成し、プラグインが送信データを積む。
    Publisher,
    /// ホストが Subscriber を作成し、受信データをプラグインへ渡す。
    Subscriber,
}

/// QoSプリセット。
#[repr(u8)]
#[derive(StableAbi, Debug, Clone, Copy, PartialEq, Eq)]
pub enum QosPreset {
    /// 最新値のみ保持（センサーデータ向け）。
    BestEffort,
    /// 全データ保証・順序保証（コマンド向け）。
    Reliable,
}

/// `update` 時のトピックデータ。受信（Host→Plugin）および送信（Plugin→Host）の両方に使う。
/// ペイロードはCDRエンコード済みバイト列。エンコード形式はプラグインとホストで合わせること。
#[repr(C)]
#[derive(StableAbi)]
pub struct TopicData {
    /// トピック名。`TopicDescriptor::name` に対応する。
    pub name: RString,
    /// CDRエンコード済みペイロード。
    pub payload: RVec<u8>,
}

/// プラグインモジュールのABI定義。
///
/// Prefixモジュール方式を採用しているため、末尾への新フィールド追加は後方互換。
#[repr(C)]
#[derive(StableAbi)]
#[sabi(kind(Prefix(prefix_ref = RobotPlugin_Ref)))]
#[sabi(missing_field(panic))]
pub struct RobotPlugin {
    /// プラグインの初期化。必要なトピック一覧を返す。
    ///
    /// - `ctx`: ホストが渡す初期化コンテキスト。
    /// - `prev_state`: 直前の `shutdown` が返した状態バイト列。初回起動時は `RNone`。
    ///
    /// 戻り値のリストにもとづきホストがDDSエンティティを作成する。
    pub init: extern "C" fn(
        ctx: &PluginContext,
        prev_state: ROption<RSlice<'_, u8>>,
    ) -> RVec<TopicDescriptor>,

    /// 制御ループの1ステップ。
    ///
    /// - `received`: ホストがSubscriberから読み取ったデータ（全件Reliable）。
    /// - `publish`: プラグインが送信したいデータを積む。ホストがPublisherへ書き込む。
    ///
    /// 戻り値: `0` = 正常、負値 = エラー（ホストはFallbackへ切り替える）。
    pub update:
        extern "C" fn(received: RSlice<'_, TopicData>, publish: &mut RVec<TopicData>) -> i32,

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
