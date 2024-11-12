use std::path::PathBuf;

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::IntoResponse,
    Json,
};
use image::ImageEncoder;
use v4l::{util::control::ControlTable, video::Capture};

use crate::{
    capture::{self, CaptureProp, Controls},
    error::AppError,
    util::open_device,
    Context,
};

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
pub struct DeviceDetail {
    pub controls: Vec<Description>,
    pub formats: Vec<FormatDesc>,
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

#[derive(Debug, PartialEq, serde::Serialize)]
pub struct FormatDesc {
    pub index: u32,
    pub description: String,
    pub fourcc: String,
    pub framesizes: Vec<Discrete>,
}

impl FormatDesc {
    fn with_fmt_disc(fmt: v4l::format::Description, framesizes: Vec<Discrete>) -> Self {
        FormatDesc {
            index: fmt.index,
            description: fmt.description,
            fourcc: fmt.fourcc.to_string(),
            framesizes,
        }
    }
}

#[derive(Debug, PartialEq, serde::Serialize)]
pub struct Discrete {
    pub width: u32,
    pub height: u32,
}

impl From<v4l::framesize::Discrete> for Discrete {
    fn from(fmt: v4l::framesize::Discrete) -> Self {
        Discrete {
            width: fmt.width,
            height: fmt.height,
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct CaptureQuery {
    pub fourcc: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub control: Option<String>,
    /// カメラの安定を待つバッファ数
    #[serde(default = "CaptureQuery::buffer_count_default")]
    pub buffer_count: u32,
    pub outfmt: OutFmt,
}

impl CaptureQuery {
    fn buffer_count_default() -> u32 {
        4
    }

    /// クエリの他、未入力の場合はデバイスデフォルトの値を使用してCapturePropを生成する
    pub fn to_prop(&self, format: v4l::format::Format, ctrls: Option<Controls>) -> CaptureProp {
        CaptureProp {
            fourcc: self.fourcc.clone().unwrap_or(format.fourcc.to_string()),
            width: self.width.unwrap_or(format.width),
            height: self.height.unwrap_or(format.height),
            controls: ctrls,
            buffer_count: self.buffer_count,
        }
    }
}

/// RAW画像の出力フォーマット
#[derive(Debug, Default, PartialEq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutFmt {
    /// 変換なし
    #[default]
    Default,
    /// Png変換
    Png,
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
    let dev = open_device(index)?;
    let cap = dev.query_controls().inspect_err(|e| {
        tracing::error!("Failed to query controls: {:?}", e);
    })?;

    let mut controls = vec![];
    for ctrl in cap {
        controls.push(Description::from(ctrl));
    }

    let mut formats = vec![];
    for fmt in dev.enum_formats().inspect_err(|e| {
        tracing::error!("Failed to query format: {:?}", e);
    })? {
        let mut dics = vec![];
        for framesize in dev.enum_framesizes(fmt.fourcc)? {
            for discrete in framesize.size.to_discrete() {
                dics.push(discrete.into());
            }
        }
        formats.push(FormatDesc::with_fmt_disc(fmt, dics));
    }
    Ok(Json(DeviceDetail { controls, formats }))
}

/// Capture image from device
pub async fn capture(
    State(context): State<Context>,
    Path(index): Path<usize>,
    query: Query<CaptureQuery>,
) -> Result<impl IntoResponse, AppError> {
    let (default_format, ctrls) = {
        let dev = open_device(index)?;
        let format = dev.format().inspect_err(|e| {
            tracing::error!("Failed to get format: {:?}", e);
        })?;

        let ctrls = if let Some(ctrl_str) = query.0.control.as_ref() {
            let req = v4l::util::control::Requests::try_from(ctrl_str.as_str())
                .map_err(|e| anyhow::anyhow!("Failed to create control requests: {:?}", e))?;
            let ctrlmap = ControlTable::from(dev.query_controls()?.as_slice());
            let def = ctrlmap.get_default(&req);
            let target = ctrlmap.get_control(&req);
            Some(Controls::new(def, target))
        } else {
            None
        };
        (format, ctrls)
    };

    let prop = query.0.to_prop(default_format, ctrls);
    prop.validate()?;
    tracing::info!("Capture: {:?}", prop);
    let format = prop.format();

    // デバイスを開く操作は1つだけしか許されないため
    // Captureは別の単一フローのルーチンで取得する
    let (tx, rx) = tokio::sync::oneshot::channel();
    let req = capture::Request::Capture {
        tx,
        format,
        device_index: index,
        buffer_count: prop.buffer_count,
        controls: prop.controls,
    };
    context.capture_tx.send(req).await.inspect_err(|e| {
        tracing::error!("Failed to send capture request: {:?}", e);
    })?;
    let mut res = rx.await.inspect_err(|e| {
        tracing::error!("Failed to receive capture response: {:?}", e);
    })??;

    let mut headers = HeaderMap::new();
    if res.format.fourcc == "MJPG" {
        headers.insert("Content-Type", "image/jpeg".parse().unwrap());
    } else if query.0.outfmt == OutFmt::Png {
        match res.format.fourcc.as_str() {
            "YUYV" => {
                headers.insert("Content-Type", "image/png".parse().unwrap());
                let rgb = crate::imgfmt::yuyv422_to_rgb(
                    &res.buffer,
                    res.format.width,
                    res.format.height,
                )
                .inspect_err(|e| {
                    tracing::error!("Failed to convert YUYV to RGB: {:?}", e);
                })?;
                let img = image::RgbImage::from_raw(res.format.width, res.format.height, rgb)
                    .unwrap();
                // clear buffer and encode PNG
                res.buffer.clear();
                let writer = std::io::BufWriter::new(&mut res.buffer);
                let enc = image::codecs::png::PngEncoder::new(writer);
                enc.write_image(
                    img.as_raw(),
                    img.width(),
                    img.height(),
                    image::ExtendedColorType::Rgb8,
                )
                .inspect_err(|e| {
                    tracing::error!("Failed to encode PNG: {:?}", e);
                })?;
            }
            _ => {
                headers.insert("Content-Type", "image/raw".parse().unwrap());
            }
        }
    } else {
        headers.insert("Content-Type", "image/raw".parse().unwrap());
    }

    Ok((headers, Body::from(res.buffer)))
}
