use std::path::PathBuf;

use axum::{extract::Path, response::IntoResponse, Json};

use crate::error::AppError;

/// V4l2 deviceの情報を格納する構造体
#[derive(Debug, serde::Serialize, PartialEq)]
struct Device {
    index: usize,
    path: PathBuf,
    cap: Capabilities,
}

/// Device capabilities with Serialize
#[derive(Debug, serde::Serialize, PartialEq)]
pub struct Capabilities {
    pub driver: String,
    pub card: String,
    pub bus: String,
    pub version: (u8, u8, u8),
    pub capabilities: String,
}

impl From<v4l::Capabilities> for Capabilities {
    fn from(caps: v4l::Capabilities) -> Self {
        Capabilities {
            driver: caps.driver,
            card: caps.card,
            bus: caps.bus,
            version: caps.version,
            capabilities: caps.capabilities.to_string(),
        }
    }
}

#[derive(Debug, PartialEq, serde::Serialize)]
/// Device control description
pub struct Description {
    pub id: u32,
    pub typ: String,
    pub name: String,
    pub minimum: i64,
    pub maximum: i64,
    pub step: u64,
    pub default: i64,
    pub flags: String,
    pub items: Option<Vec<(u32, String)>>,
}

impl From<v4l::control::Description> for Description {
    fn from(ctrl: v4l::control::Description) -> Self {
        Self {
            id: ctrl.id,
            typ: ctrl.typ.to_string(),
            name: ctrl.name,
            minimum: ctrl.minimum,
            maximum: ctrl.maximum,
            step: ctrl.step,
            default: ctrl.default,
            flags: ctrl.flags.to_string(),
            items: ctrl.items.map(|items| {
                items
                    .iter()
                    .map(|(id, item)| (*id, item.to_string()))
                    .collect()
            }),
        }
    }
}

/// List all v4l2 devices
pub async fn list() -> Result<impl IntoResponse, AppError> {
    use v4l::context;
    let mut res = vec![];
    for node in context::enum_devices() {
        let dev = v4l::Device::with_path(node.path()).inspect_err(|e| {
            tracing::error!("Failed to open device [{}]: {}", node.path().display(), e)
        })?;
        let cap = dev.query_caps().inspect_err(|e| {
            tracing::error!("Failed to query capabilities: {:?}", e);
        })?;
        res.push(Device {
            index: node.index(),
            path: node.path().to_path_buf(),
            cap: Capabilities::from(cap),
        });
    }
    res.sort_by(|a, b| a.index.cmp(&b.index));
    Ok(Json(res))
}

// get device and show controls
pub async fn device(Path(index): Path<usize>) -> Result<impl IntoResponse, AppError> {
    let dev = v4l::Device::new(index).inspect_err(|e| {
        tracing::error!("Failed to open device: {:?}", e);
    })?;
    let cap = dev.query_controls().inspect_err(|e| {
        tracing::error!("Failed to query controls: {:?}", e);
    })?;

    let mut res = vec![];
    for ctrl in cap {
        res.push(Description::from(ctrl));
    }
    Ok(Json(res))
}
