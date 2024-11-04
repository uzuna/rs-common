#![cfg(target_arch = "wasm32")]

extern crate wasm_bindgen_test;

use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
#[ignore = "テストはタイムアウトするので無効化"]
async fn test_image_load() -> std::result::Result<(), JsValue> {
    wasm_util::panic::set_panic_hook();
    let loader = wasm_util::image::ImageLoader::new("https://interactive-examples.mdn.mozilla.net/media/cc0-images/grapefruit-slice-332-332.jpg")?;
    let img = loader.await?;
    assert_eq!(img.width(), 332);
    assert_eq!(img.height(), 332);
    Ok(())
}
