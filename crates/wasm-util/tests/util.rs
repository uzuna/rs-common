//! Test suite for the Web and headless browsers.

#![cfg(target_arch = "wasm32")]

extern crate wasm_bindgen_test;

use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
async fn test_element() -> std::result::Result<(), JsValue> {
    wasm_util::panic::set_panic_hook();
    let body = wasm_util::util::get_body()?;
    let div = wasm_util::util::create_element::<web_sys::HtmlElement>("div")?;
    div.set_inner_html("Hello, World!");
    div.set_id("test");
    body.append_child(&div)?;

    let div2 = wasm_util::util::get_element::<web_sys::HtmlElement>("test")?;
    assert_eq!(div2.inner_html(), "Hello, World!");
    Ok(())
}

#[wasm_bindgen_test]
async fn test_performance() -> std::result::Result<(), JsValue> {
    wasm_util::panic::set_panic_hook();
    let performance = wasm_util::util::get_performance()?;
    let _ = performance.now();
    Ok(())
}
