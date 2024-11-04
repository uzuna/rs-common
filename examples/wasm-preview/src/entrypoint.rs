use wasm_bindgen::prelude::*;
use wasm_util::{error::*, info};

/// モジュールの初期化処理
#[wasm_bindgen(start)]
pub fn init() -> Result<()> {
    wasm_util::panic::set_panic_hook();
    Ok(())
}

/// wasmのエントリーポイントとして定義
#[wasm_bindgen]
pub fn start() -> std::result::Result<(), JsValue> {
    info!("Hello, wasm!");
    wasm_util::error!("This is a error.");
    Ok(())
}
