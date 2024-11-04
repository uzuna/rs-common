use wasm_bindgen::prelude::*;

use crate::error::Result;

/// windowオブジェクトを取得するラッパー
pub fn get_window() -> Result<web_sys::Window> {
    web_sys::window().ok_or(JsError::new("window is None"))
}

/// documentオブジェクトを取得するラッパー
pub fn get_document() -> Result<web_sys::Document> {
    get_window()?
        .document()
        .ok_or(JsError::new("document is None"))
}

/// 指定idのエレメントを取得するラッパー
pub fn get_element<T>(id: impl AsRef<str>) -> Result<T>
where
    T: wasm_bindgen::JsCast,
{
    let id = id.as_ref();
    get_document()?
        .get_element_by_id(id)
        .ok_or(JsError::new(&format!("Failed to get element: {id}")))?
        .dyn_into::<T>()
        .map_err(|_| JsError::new(&format!("Failed to convert Element: {id}")))
}

/// エレメントを作成のラッパー
pub fn create_element<T>(tag: impl AsRef<str>) -> Result<T>
where
    T: wasm_bindgen::JsCast,
{
    get_document()?
        .create_element(tag.as_ref())
        .map_err(|_| JsError::new("cannot create element"))?
        .dyn_into::<T>()
        .map_err(|_| JsError::new("cannot convert to HtmlElement"))
}

/// bodyエレメントを取得するラッパー
pub fn get_body() -> Result<web_sys::HtmlElement> {
    get_document()?
        .body()
        .ok_or(JsError::new("body is None"))?
        .dyn_into::<web_sys::HtmlElement>()
        .map_err(|_| JsError::new("cannot convert to HtmlElement"))
}

/// performanceオブジェクトを取得するラッパー
pub fn get_performance() -> Result<web_sys::Performance> {
    get_window()?
        .performance()
        .ok_or(JsError::new("Failed to get performance"))
}
