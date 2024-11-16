use jetson_pixfmt::pixfmt::CsiPixelFormat;
use tokio::sync::{mpsc, oneshot};
use v4l::Control;

use crate::capture::CaptureResponse;

pub trait Context {
    fn capture_tx(&self) -> mpsc::Sender<Request>;
}

pub enum Request {
    Capture {
        tx: oneshot::Sender<Result<CaptureResponse, anyhow::Error>>,
        args: CaptureArgs,
    },
    CaptureStack {
        tx: oneshot::Sender<Result<CaptureResponse, anyhow::Error>>,
        args: CaptureArgs,
        stack_count: usize,
        csv_format: CsiPixelFormat,
    },
}

/// カメラのコントロールの設定
#[derive(Debug)]
pub struct Controls {
    pub def: Vec<Control>,
    pub target: Vec<Control>,
}

impl Controls {
    pub fn new(def: Vec<Control>, target: Vec<Control>) -> Self {
        Controls { def, target }
    }
}

pub struct CaptureArgs {
    pub device_index: usize,
    pub format: v4l::format::Format,
    pub buffer_count: u32,
    pub controls: Option<Controls>,
}
