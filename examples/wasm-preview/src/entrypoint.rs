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

    wasm_bindgen_futures::spawn_local(async {
        match load_image().await {
            Ok(elapsed) => info!("Image loaded in {} ms", elapsed),
            Err(e) => wasm_util::error!("Failed to load image: {:?}", e),
        }
    });
    Ok(())
}

// 画像を読み込む非同期関数
async fn load_image() -> Result<f64> {
    let p = wasm_util::util::get_performance()?;
    let start = p.now();

    let loader = wasm_util::image::ImageLoader::new(
        "https://rustacean.net/assets/rustacean-flat-happy.png",
    )?;
    let img = loader.await?;
    info!("Image loaded: {}x{}", img.width(), img.height());
    let el = wasm_util::util::get_element::<web_sys::HtmlElement>("image-target")?;
    el.append_child(img.as_ref())
        .map_err(|_| JsError::new("failed to append image"))?;
    let elapsed = p.now() - start;
    Ok(elapsed)
}
