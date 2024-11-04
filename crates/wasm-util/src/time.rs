//! タイマー関連のユーティリティ

use std::{
    future::Future,
    pin::Pin,
    rc::Rc,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    task::{Context, Poll},
    time::Duration,
};

use futures_util::{future::FusedFuture, stream::FusedStream, Stream};
use wasm_bindgen::prelude::*;

use crate::{error::*, util::get_window};

/// 一定時間後に解決するFuture
///
/// set_timeoutを利用した非周期タイマー
pub struct Timeout {
    millis: i32,
    id: Option<i32>,
    closure: Option<Closure<dyn FnMut()>>,
}

impl Timeout {
    pub fn new(millis: i32) -> Self {
        Self {
            millis,
            id: None,
            closure: None,
        }
    }

    pub fn cancel(&mut self) {
        if let Some(id) = self.id.take() {
            get_window().unwrap_throw().clear_timeout_with_handle(id);
        }
    }
}

impl Future for Timeout {
    type Output = Result<()>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        if let Some(_id) = self.id.take() {
            Poll::Ready(Ok(()))
        } else {
            let waker = cx.waker().clone();
            let closure = Closure::once(move || {
                waker.wake_by_ref();
            });
            let id = get_window()?
                .set_timeout_with_callback_and_timeout_and_arguments_0(
                    closure.as_ref().unchecked_ref(),
                    self.millis,
                )
                .unwrap_throw();
            self.id = Some(id);
            self.closure = Some(closure);
            Poll::Pending
        }
    }
}

impl FusedFuture for Timeout {
    fn is_terminated(&self) -> bool {
        self.id.is_none()
    }
}

impl Drop for Timeout {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// [Timeout::new]のエイリアス
pub async fn sleep(dur: Duration) -> Result<()> {
    Timeout::new(dur.as_millis() as i32).await
}

/// 指定周期で解決するStream
///
/// set_intervalを利用した周期タイマー
pub struct Interval {
    millis: i32,
    id: Option<i32>,
    closure: Option<Closure<dyn FnMut()>>,
    value: Rc<AtomicBool>,
    closed: bool,
}

impl Interval {
    pub fn new(millis: i32) -> Self {
        Self {
            millis,
            id: None,
            closure: None,
            value: Rc::new(AtomicBool::new(false)),
            closed: false,
        }
    }

    /// 指定時間ごとに呼び出される周期タイマーを作成する
    pub fn with_duration(dur: Duration) -> Self {
        Self::new(dur.as_millis() as i32)
    }

    /// Intervalをキャンセルする
    pub fn cancel(&mut self) {
        self.closed = true;
        if let Some(id) = self.id.take() {
            get_window().unwrap_throw().clear_interval_with_handle(id);
        }
    }
}

impl Stream for Interval {
    type Item = Result<()>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        if self.closed {
            Poll::Ready(None)
        } else if self.value.load(Ordering::Relaxed) {
            self.value.store(false, Ordering::Relaxed);
            Poll::Ready(Some(Ok(())))
        } else if self.id.is_some() {
            Poll::Pending
        } else {
            let waker = cx.waker().clone();
            let b = self.value.clone();
            let closure = Closure::new(move || {
                b.store(true, Ordering::Relaxed);
                waker.wake_by_ref();
            });
            let id = get_window()?
                .set_interval_with_callback_and_timeout_and_arguments_0(
                    closure.as_ref().unchecked_ref(),
                    self.millis,
                )
                .unwrap_throw();
            self.id = Some(id);
            self.closure = Some(closure);
            Poll::Pending
        }
    }
}

impl FusedStream for Interval {
    fn is_terminated(&self) -> bool {
        self.closed
    }
}

impl Drop for Interval {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// request animation frameの周期を待つTicker
pub struct AnimationTicker {
    timestamp: Rc<AtomicU64>,
}

impl AnimationTicker {
    /// 次のアニメーションフレームを待つ
    pub fn tick(&mut self) -> AnimationInstanct {
        AnimationInstanct::new(self.timestamp.clone())
    }

    /// 最後のタイムスタンプを取得
    pub fn last_timestamp(&self) -> f64 {
        f64::from_bits(self.timestamp.load(Ordering::Relaxed))
    }
}

impl Default for AnimationTicker {
    fn default() -> Self {
        Self {
            timestamp: Rc::new(AtomicU64::new(0)),
        }
    }
}

/// requestAnimationFrameを待つFutureの実装
pub struct AnimationInstanct {
    closure: Option<Closure<dyn FnMut(f64)>>,
    handle: Option<i32>,
    timestamp: Rc<AtomicU64>,
}

impl AnimationInstanct {
    fn new(timestamp: Rc<AtomicU64>) -> Self {
        Self {
            closure: None,
            handle: None,
            timestamp,
        }
    }

    fn cancel(&mut self) {
        if let Some(handle) = self.handle.take() {
            cancel_animation_frame_inner(handle).unwrap_throw();
        }
    }
}

impl Future for AnimationInstanct {
    type Output = Result<f64>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        use std::borrow::BorrowMut;
        // wakerが呼ばれたら基本的にはタスクが終了しているはず
        if let Some(_handle) = self.handle.take() {
            let ts = f64::from_bits(self.timestamp.load(Ordering::Relaxed));
            Poll::Ready(Ok(ts))
        } else {
            // await callされたたらタスクを開始
            let waker = cx.waker().clone();
            let mut ts = self.timestamp.clone();
            let closure = Closure::wrap(Box::new(move |timestamp| {
                ts.borrow_mut()
                    .store(f64::to_bits(timestamp), Ordering::Relaxed);
                waker.wake_by_ref();
            }) as Box<dyn FnMut(f64)>);
            self.handle = Some(request_animation_frame_inner(&closure)?);
            self.closure = Some(closure);
            Poll::Pending
        }
    }
}

impl FusedFuture for AnimationInstanct {
    fn is_terminated(&self) -> bool {
        self.handle.is_none()
    }
}

impl Drop for AnimationInstanct {
    fn drop(&mut self) {
        self.cancel();
    }
}

// 次のアニメーションフレームのcallbackを登録
fn request_animation_frame_inner(closure: &Closure<dyn FnMut(f64)>) -> Result<i32> {
    get_window()?
        .request_animation_frame(closure.as_ref().unchecked_ref())
        .map_err(|_| JsError::new("Failed request animation frame"))
}

// 再生リクエストをキャンセル
fn cancel_animation_frame_inner(handle: i32) -> std::result::Result<(), JsValue> {
    get_window()?.cancel_animation_frame(handle)
}
