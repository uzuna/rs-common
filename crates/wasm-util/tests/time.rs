#![cfg(target_arch = "wasm32")]

extern crate wasm_bindgen_test;

use futures::select;
use std::time::Duration;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
async fn test_sleep() -> std::result::Result<(), JsValue> {
    use futures::FutureExt;
    use wasm_util::time::sleep;

    wasm_util::panic::set_panic_hook();

    let p = wasm_util::util::get_performance()?;
    let start = p.now();
    sleep(Duration::from_millis(100)).await?;
    assert!(p.now() - start >= 100.0);

    // t1が先に解決して300ms経過しないことを確認
    let t1 = sleep(Duration::from_millis(100)).fuse();
    let t2 = sleep(Duration::from_millis(200)).fuse();

    select! {
        _ = {t1} => {},
        _ = {t2} => {},
    }

    assert!(p.now() - start >= 100.0 && p.now() - start < 300.0);
    Ok(())
}

#[wasm_bindgen_test]
async fn test_interval() -> std::result::Result<(), JsValue> {
    use futures::StreamExt;
    wasm_util::panic::set_panic_hook();
    let p = wasm_util::util::get_performance()?;
    let start = p.now();

    let mut count = 0;
    let mut ticker = wasm_util::time::Interval::with_duration(Duration::from_millis(10));
    while let Some(_) = ticker.next().await {
        count += 1;
        if p.now() - start >= 100.0 {
            break;
        }
    }
    assert_eq!(count, 10);
    Ok(())
}

#[wasm_bindgen_test]
async fn test_animation_ticker() -> std::result::Result<(), JsValue> {
    wasm_util::panic::set_panic_hook();
    let p = wasm_util::util::get_performance()?;
    let start = p.now();

    let mut count = 0;
    let mut ticker = wasm_util::time::AnimationTicker::default();

    while let Ok(_) = ticker.tick().await {
        count += 1;
        if p.now() - start >= 100.0 {
            break;
        }
    }
    // 100ms / 16ms(60Hz) = 6.25
    assert!(count >= 6);
    Ok(())
}
