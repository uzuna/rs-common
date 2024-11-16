use tokio::sync::{mpsc, oneshot};
use v4l::Control;

use crate::capture::CaptureResponse;

pub trait Context {
    fn capture_tx(&self) -> mpsc::Sender<Request>;
}

pub enum Request {
    Capture {
        tx: oneshot::Sender<Result<CaptureResponse, anyhow::Error>>,
        device_index: usize,
        format: v4l::Format,
        buffer_count: u32,
        controls: Option<Controls>,
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