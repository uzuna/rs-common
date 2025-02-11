use mls_mpm::Sim;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use wasm_util::{error::*, info, time::AnimationTicker};

/// モジュールの初期化処理
#[wasm_bindgen(start)]
pub fn init() -> Result<()> {
    wasm_util::panic::set_panic_hook();
    Ok(())
}

/// wasmのエントリーポイントとして定義
#[wasm_bindgen]
pub fn start() -> std::result::Result<(), JsValue> {
    spawn_local(async {
        let config = mls_mpm::SimConfig::new(100, 10);
        let mut sim = Sim::<f32>::init(config);
        let mut t = AnimationTicker::default();
        while let Ok(i) = t.tick().await {
            let dt_sec = i as f32 / 1000.0;
            sim.simulate(dt_sec);
            info!("tick {dt_sec:?}");
        }
    });
    Ok(())
}
