#![cfg(target_arch = "wasm32")]

extern crate wasm_bindgen_test;

use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
async fn test_console() -> std::result::Result<(), JsValue> {
    wasm_util::info!("test info {}", 1);
    wasm_util::error!("test error {}", 2);
    Ok(())
}

#[wasm_bindgen_test]
#[ignore = "alert is not supported in headless browsers"]
async fn test_alert() -> std::result::Result<(), JsValue> {
    wasm_util::alert!("test alert {}", 3);
    Ok(())
}
