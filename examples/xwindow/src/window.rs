//! 実行プラットフォーム。ディスプレイ取得など
use std::ffi::CString;
use std::mem;
use std::os::raw::*;
use std::ptr;

use anyhow::{anyhow, Result};
use x11_dl::xlib;
pub use xegl::{Config, EglContext};

/// Windowを生成するためのビルダー
#[derive(Debug, PartialEq)]
pub struct WindowBuilder {
    title: String,
    width: u32,
    height: u32,
    enable_ui_event: bool,
    modal_window: bool,
    egl_config: Config,
}

impl WindowBuilder {
    pub fn new(title: &str, width: u32, height: u32) -> Self {
        Self {
            title: title.to_string(),
            width,
            height,
            enable_ui_event: false,
            modal_window: false,
            egl_config: Config::default(),
        }
    }

    /// マウス、キーボードイベントを無効にする
    pub fn enable_ui_event(&mut self) -> &mut Self {
        self.enable_ui_event = true;
        self
    }

    /// モーダルウィンドウとして表示、タイトルバーを非表示で全画面表示
    pub fn modal_window(&mut self) -> &mut Self {
        self.modal_window = true;
        self
    }

    pub fn build(self) -> Result<Window> {
        Window::new(self)
    }
}

/// Window デバイスやライブラリ依存を覆い隠すための抽象化層
pub struct Window {
    /// xlib API root
    xlib: xlib::Xlib,

    /// display pointer
    display: *mut xlib::_XDisplay,

    /// window id
    #[allow(dead_code)]
    window: c_ulong,

    // X11 window event management
    wm: WindowManageEventParser,
    event: xlib::XEvent,

    /// has egl context
    egl_context: EglContext,
}

impl Window {
    fn new(builder: WindowBuilder) -> Result<Self> {
        unsafe {
            // Load Xlib library.
            let xl = xlib::Xlib::open()?;

            // Open display connection.
            let display = (xl.XOpenDisplay)(ptr::null());
            if display.is_null() {
                return Err(anyhow!("XOpenDisplay failed"));
            }

            let screen = (xl.XDefaultScreen)(display);
            let root = (xl.XRootWindow)(display, screen);

            let mut attributes: xlib::XSetWindowAttributes = mem::zeroed();
            attributes.background_pixel = (xl.XWhitePixel)(display, screen);

            if !builder.enable_ui_event {
                // マウスやキーボードのイベントを無視する
                attributes.event_mask =
                    xlib::ExposureMask | xlib::PointerMotionMask | xlib::KeyPressMask;
            }

            // Create window.
            let window = (xl.XCreateWindow)(
                display,
                root,
                0,
                0,
                builder.width,
                builder.height,
                0,
                0,
                xlib::InputOutput as c_uint,
                ptr::null_mut(),
                xlib::CWBackPixel,
                &mut attributes,
            );

            // UIイベントが有効ならキー入力を受け付ける
            if builder.enable_ui_event {
                (xl.XSelectInput)(display, window, xlib::KeyReleaseMask | xlib::KeyPressMask);
            }

            if builder.modal_window {
                // ウィンドウをルートウィンドウ上で移動できない用に配置する
                // タイトルバーなどを表示せず画面全体を描画領域で覆う設定
                let mut xattrs: xlib::XSetWindowAttributes = mem::zeroed();
                xattrs.override_redirect = xlib::True;
                (xl.XChangeWindowAttributes)(
                    display,
                    window,
                    xlib::CWOverrideRedirect,
                    &mut xattrs,
                );
            }

            // Set window title.
            let title_str = CString::new(builder.title)?;
            (xl.XStoreName)(display, window, title_str.as_ptr() as *mut c_char);

            // EGL取得前に何故かこれを実行しないとcreate_window_surfaceが失敗する
            // TODO 理由の調査
            let wm = WindowManageEventParser::new(&xl, display, window);

            // window表示
            (xl.XMapWindow)(display, window);

            // X11内のイベントを吸い上げる口
            // これを破棄するとイベント処理が出来ずdisplayが更新されなくなる
            let event: xlib::XEvent = mem::zeroed();

            // gl命令を有効にするためにEGLコンテキストを生成
            let egl_context =
                EglContext::new(window as xegl::NativeWindowType, &builder.egl_config)?;

            Ok(Self {
                xlib: xl,
                display,
                window,
                wm,
                event,
                egl_context,
            })
        }
    }

    /// TODO しばらく動かしているとイベントが詰まるのか何も取れなくなる
    /// 時間があるときに調査する
    pub fn read_event(&mut self) -> Option<WindowEventMsg> {
        unsafe {
            while (self.xlib.XPending)(self.display) == xlib::True {
                (self.xlib.XNextEvent)(self.display, &mut self.event);

                if let Some(x) = self.wm.read_event(self.event) {
                    return Some(x);
                }
            }
            None
        }
    }

    /// 画面のバッファを入れ替える
    pub fn gl_swap_window(&self) -> anyhow::Result<()> {
        Ok(self.egl_context.swap_buffers()?)
    }

    pub fn egl_ctx(&self) -> &EglContext {
        &self.egl_context
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            (self.xlib.XCloseDisplay)(self.display);
        }
    }
}

/// Windowで利用する可能性のあるイベントのリスト
#[derive(Debug, PartialEq, Eq)]
pub enum WindowEventMsg {
    KeyPress(u32),
    DeleteWindow,
}

/// WindowManageEventParser
struct WindowManageEventParser {
    protocols: xlib::Atom,
    delete_window: xlib::Atom,
}

impl WindowManageEventParser {
    fn new(xl: &xlib::Xlib, display: *mut xlib::_XDisplay, window: c_ulong) -> Self {
        unsafe {
            // Hook close requests.
            let wm_protocols_str = CString::new("WM_PROTOCOLS").expect("WM_PROTOCOLS is failed");
            let wm_delete_window_str =
                CString::new("WM_DELETE_WINDOW").expect("WM_DELETE_WINDOW is failed");
            let wm_protocols = (xl.XInternAtom)(display, wm_protocols_str.as_ptr(), xlib::False);
            let wm_delete_window =
                (xl.XInternAtom)(display, wm_delete_window_str.as_ptr(), xlib::False);

            let mut protocols = [wm_delete_window];

            (xl.XSetWMProtocols)(
                display,
                window,
                protocols.as_mut_ptr(),
                protocols.len() as c_int,
            );
            Self {
                protocols: wm_protocols,
                delete_window: wm_delete_window,
            }
        }
    }

    fn read_event(&self, event: xlib::XEvent) -> Option<WindowEventMsg> {
        match event.get_type() {
            xlib::ClientMessage => {
                let xclient = xlib::XClientMessageEvent::from(event);
                if xclient.message_type == self.protocols && xclient.format == 32 {
                    let protocol = xclient.data.get_long(0) as xlib::Atom;
                    if protocol == self.delete_window {
                        Some(WindowEventMsg::DeleteWindow)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            xlib::KeyPress => {
                let xkey = xlib::XKeyEvent::from(event);
                Some(WindowEventMsg::KeyPress(xkey.keycode))
            }
            _ => None,
        }
    }
}
