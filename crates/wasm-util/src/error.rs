use wasm_bindgen::JsError;

/// エラーはJsErrorを使う
pub type Result<T> = std::result::Result<T, JsError>;
