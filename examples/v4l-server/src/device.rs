use std::path::PathBuf;

use axum::{response::IntoResponse, Json};

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

/// List all v4l2 devices
pub async fn list() -> impl IntoResponse {
    use v4l::context;
    let mut res = vec![];
    for node in context::enum_devices() {
        let dev = v4l::Device::with_path(node.path()).unwrap();
        let cap = dev.query_caps().unwrap();
        res.push(Device {
            index: node.index(),
            path: node.path().to_path_buf(),
            cap: Capabilities::from(cap),
        });
    }
    res.sort_by(|a, b| a.index.cmp(&b.index));
    Json(res)
}
