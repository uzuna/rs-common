//! サンプルプラグイン実装。
//!
//! Phase 2 の検証用として以下の動作を実装する:
//! - 通常動作: 受信データを無視し、空の送信リストを返す。
//! - パニックモード: 環境変数 `PLUGIN_SHOULD_PANIC=1` が設定されている場合、
//!   `update` 内でパニックを発生させる。ただし `catch_unwind` でラップされているため
//!   ホストプロセスは停止せず、エラーコード `-1` が返る。

use std::sync::Mutex;

use abi_stable::{
    export_root_module,
    prefix_type::PrefixTypeTrait,
    std_types::{ROption, RSlice, RVec},
};
use safety_plugin_common::{
    PluginContext, QosPreset, RobotPlugin, RobotPlugin_Ref, TopicData, TopicDescriptor,
    TopicDirection,
};

// プラグイン内部状態（ホットリロード時に引き継ぐ）
#[derive(Default)]
struct PluginState {
    /// 累積ステップ数。状態引き継ぎ検証に使う。
    step_count: u64,
}

static STATE: Mutex<PluginState> = Mutex::new(PluginState { step_count: 0 });

/// abi_stable がこの関数をエントリポイントとして認識する。
#[export_root_module]
fn get_library() -> RobotPlugin_Ref {
    RobotPlugin {
        init,
        update,
        shutdown,
    }
    .leak_into_prefix()
}

/// 初期化。前回の状態があれば復元し、必要トピックの記述子リストを返す。
extern "C" fn init(
    _ctx: &PluginContext,
    prev_state: ROption<RSlice<'_, u8>>,
) -> RVec<TopicDescriptor> {
    let mut state = STATE.lock().unwrap();

    // 前回の状態（バイト列）を復元する
    if let abi_stable::std_types::RSome(bytes) = prev_state {
        if bytes.len() == 8 {
            let arr: [u8; 8] = bytes[..8].try_into().unwrap_or([0u8; 8]);
            state.step_count = u64::from_le_bytes(arr);
        }
    }

    eprintln!(
        "[example-plugin] init: step_count={} から再開",
        state.step_count
    );

    // このプラグインが使用するトピックを宣言する
    // Phase 4でDDS統合時に実際のトピック名を設定する
    RVec::from(vec![
        TopicDescriptor {
            name: "robot/sensor".into(),
            direction: TopicDirection::Subscriber,
            qos: QosPreset::Reliable,
        },
        TopicDescriptor {
            name: "robot/cmd_vel".into(),
            direction: TopicDirection::Publisher,
            qos: QosPreset::Reliable,
        },
    ])
}

/// 制御ループの1ステップ。
///
/// 環境変数 `PLUGIN_SHOULD_PANIC=1` が設定されている場合は意図的にパニックを発生させる。
/// パニックは `catch_unwind` でラップされているため、ホストへはエラーコード `-1` が返る。
extern "C" fn update(received: RSlice<'_, TopicData>, publish: &mut RVec<TopicData>) -> i32 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        update_inner(received, publish)
    }));
    match result {
        Ok(v) => v,
        Err(_) => {
            eprintln!("[example-plugin] panic を捕捉しました。エラーコード -1 を返します");
            -1
        }
    }
}

/// update の実処理。パニックを発生させる可能性がある。
fn update_inner(received: RSlice<'_, TopicData>, _publish: &mut RVec<TopicData>) -> i32 {
    // パニックモード: 環境変数で制御
    if std::env::var("PLUGIN_SHOULD_PANIC").as_deref() == Ok("1") {
        panic!("意図的なパニック（Phase 2 検証用）");
    }

    let mut state = STATE.lock().unwrap();
    state.step_count += 1;

    if !received.is_empty() {
        eprintln!(
            "[example-plugin] step={}: {}件のメッセージを受信",
            state.step_count,
            received.len()
        );
    }

    0
}

/// 終了処理。内部状態をバイト列として返す。
extern "C" fn shutdown() -> RVec<u8> {
    let state = STATE.lock().unwrap();
    eprintln!(
        "[example-plugin] shutdown: step_count={} を保存します",
        state.step_count
    );
    // step_count を little-endian 8バイトとして保存
    RVec::from(state.step_count.to_le_bytes().to_vec())
}
