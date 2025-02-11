use khronos_egl as egl;

pub mod error;
use error::*;

pub use egl::NativeWindowType;

// EGLのインスタンスの型定義。featureで切り替える
#[cfg(feature = "static")]
pub type EglInstance = egl::Instance<egl::Static>;
#[cfg(feature = "dynamic")]
pub type EglInstance = egl::DynamicInstance<egl::EGL1_4>;

/// EGLのフレームバッファに関する設定
#[derive(Debug, PartialEq)]
pub struct ConfigAttrib {
    pub name: egl::Int,
    pub value: egl::Int,
}

impl ConfigAttrib {
    fn new(name: egl::Int, value: egl::Int) -> Self {
        ConfigAttrib { name, value }
    }

    /// Red, Green, Blue, Alphaのビット数を指定する
    pub fn red(size: egl::Int) -> Self {
        ConfigAttrib::new(egl::RED_SIZE, size)
    }

    pub fn green(size: egl::Int) -> Self {
        ConfigAttrib::new(egl::GREEN_SIZE, size)
    }

    pub fn blue(size: egl::Int) -> Self {
        ConfigAttrib::new(egl::BLUE_SIZE, size)
    }

    pub fn alpha(size: egl::Int) -> Self {
        ConfigAttrib::new(egl::ALPHA_SIZE, size)
    }

    /// マルチサンプルの有効無効
    ///
    /// エッジをなめらかにするMSAA(マルチサンプルエイリアシング)のために有効にする
    /// glEnable(GL_MULTISAMPLE)
    /// glEnable(GL_SAMPLE_ALPHA_TO_COVERAGE)
    pub fn sample_bufers(enable: bool) -> Self {
        ConfigAttrib::new(egl::SAMPLE_BUFFERS, if enable { 1 } else { 0 })
    }

    /// マルチサンプルのサンプル数
    pub fn samples(size: egl::Int) -> Self {
        ConfigAttrib::new(egl::SAMPLES, size)
    }
}

/// EGLContextの設定であるConfigAttribのリスト
#[derive(Debug, PartialEq)]
pub struct Config {
    pub attribs: Vec<ConfigAttrib>,
}

impl Config {
    // binding先に渡すための配列にする
    fn as_vec(&self) -> Vec<egl::Int> {
        // 並びはname, value, name, value, ... 最後にEGL_NONEで終わる
        let mut result = Vec::with_capacity(self.attribs.len() * 2 + 1);
        for a in &self.attribs {
            result.push(a.name);
            result.push(a.value);
        }
        result.push(egl::NONE);
        result
    }
}

impl Default for Config {
    // lovot-eyeの設定を参考に設定
    fn default() -> Self {
        Config {
            attribs: vec![
                // 8bitのRGBA
                ConfigAttrib::red(8),
                ConfigAttrib::green(8),
                ConfigAttrib::blue(8),
                ConfigAttrib::alpha(8),
                // アンチエイリアスのためのマルチサンプル
                ConfigAttrib::sample_bufers(true),
                ConfigAttrib::samples(4),
            ],
        }
    }
}

/// EGLのディスプレイデバイスを管理する構造体
#[derive(Debug)]
pub struct EGLDisplay {
    egl: EglInstance,
    display: egl::Display,
}

impl EGLDisplay {
    /// Initialize EGL Display
    pub fn new() -> Result<EGLDisplay> {
        #[cfg(feature = "static")]
        let egl = egl::Instance::new(egl::Static);
        #[cfg(feature = "dynamic")]
        let egl = unsafe { EglInstance::load_required().expect("Initialize EGL Context") };
        let display = unsafe {
            egl.get_display(egl::DEFAULT_DISPLAY)
                .ok_or(Error::GetDisplay)
        }?;

        egl.initialize(display)?;
        Ok(EGLDisplay { egl, display })
    }
    // x11のEGLサポート状況を確認する
    pub fn check_x11_support(&self) -> bool {
        let dp_extensions = {
            let p = self.egl.query_string(None, egl::EXTENSIONS).unwrap();
            let list = String::from_utf8(p.to_bytes().to_vec()).unwrap_or_else(|_| String::new());
            list.split(' ').map(|e| e.to_string()).collect::<Vec<_>>()
        };
        let has_dp_extension = |e: &str| dp_extensions.iter().any(|s| s == e);
        has_dp_extension("EGL_EXT_platform_x11") || has_dp_extension("EGL_KHR_platform_x11")
    }
}

impl Drop for EGLDisplay {
    fn drop(&mut self) {
        self.egl
            .terminate(self.display)
            .expect("failed to terminate EGL");
    }
}

/// EGLのContextを管理する構造体
///
/// このコンテキストによってOpenGLのAPIへのアクセスが可能になる。  
#[derive(Debug)]
pub struct EglContext {
    display: EGLDisplay,
    context: egl::Context,
    surface: Option<egl::Surface>,
}

impl EglContext {
    /// XlibのWindowを利用してEGL Contextを作成する
    ///
    /// # Safety
    ///
    /// ユーザーがWindowハンドルが正しいものであることを保証する必要がある
    pub unsafe fn new(window: egl::NativeWindowType, configs: &Config) -> Result<Self> {
        let display = EGLDisplay::new()?;
        if !display.check_x11_support() {
            return Err(Error::ops("X11 is not supported"));
        }
        let (context, config) = Self::create_context(&display, configs)?;

        let egl = &display.egl;
        // Create Surface
        let surface = egl
            .create_window_surface(display.display, config, window, None)
            .map_err(|_| Error::ops("unable to create an EGL surface"))?;

        // Make current
        egl.make_current(display.display, Some(surface), Some(surface), Some(context))
            .map_err(|_| Error::ops("unable to make current"))?;

        // 画面の交換バッファは1つだけ
        egl.swap_interval(display.display, 1)?;

        Ok(EglContext {
            display,
            context,
            surface: Some(surface),
        })
    }

    fn create_context(
        display: &EGLDisplay,
        configs: &Config,
    ) -> Result<(egl::Context, egl::Config)> {
        let config = display
            .egl
            .choose_first_config(display.display, configs.as_vec().as_slice())
            .map_err(|_| Error::ops("unable to choose an EGL configuration"))?
            .ok_or(Error::ops("no EGL configuration found"))?;

        // Create Context and maie it current
        // GLES2のAPIを使うことをAttirbuteで指定
        let context_attributes = [egl::CONTEXT_MAJOR_VERSION, 2, egl::NONE];
        let context = display
            .egl
            .create_context(display.display, config, None, &context_attributes)
            .map_err(|_| Error::ops("unable to create an EGL context"))?;
        Ok((context, config))
    }

    pub fn no_gui(configs: &Config) -> Result<Self> {
        let display = EGLDisplay::new()?;
        let (context, _config) = Self::create_context(&display, configs)?;

        // noGUIの場合はサーフェスは作らない
        // サーフェスがない場合はデフォルトのフレームバッファも作られない。
        if display
            .egl
            .make_current(display.display, None, None, Some(context))
            .is_err()
        {
            return Err(Error::ops("failed EGL make_current"));
        }

        Ok(EglContext {
            display,
            context,
            surface: None,
        })
    }

    /// EGLのインスタンスを取得する
    pub fn egl(&self) -> &EglInstance {
        &self.display.egl
    }

    /// EGLの情報を表示する
    pub fn eglinfo(&self) -> Result<EglInfo> {
        EglInfo::query(self)
    }

    pub fn swap_buffers(&self) -> Result<()> {
        if let Some(ref surface) = self.surface {
            Ok(self
                .display
                .egl
                .swap_buffers(self.display.display, *surface)?)
        } else {
            Err(Error::ops("no surface"))
        }
    }
}

impl Drop for EglContext {
    fn drop(&mut self) {
        if let Some(surface) = self.surface {
            self.display
                .egl
                .destroy_surface(self.display.display, surface)
                .expect("failed to destroy surface");
        }
        self.display
            .egl
            .destroy_context(self.display.display, self.context)
            .expect("failed to destroy context");
    }
}

/// EGLに関するバージョン情報
#[derive(Debug, Clone)]
pub struct EglInfo {
    pub vender: String,
    pub api: String,
    pub version: String,
    pub extensions: Vec<String>,
}

impl EglInfo {
    fn new(
        vender: impl Into<String>,
        api: impl Into<String>,
        version: impl Into<String>,
        extensions: Vec<String>,
    ) -> Self {
        EglInfo {
            vender: vender.into(),
            api: api.into(),
            version: version.into(),
            extensions,
        }
    }

    fn query(ctx: &EglContext) -> Result<Self> {
        let egl = ctx.egl();
        let display = Some(ctx.display.display);
        let vender = egl.query_string(display, egl::VENDOR)?.to_string_lossy();
        let api = egl
            .query_string(display, egl::CLIENT_APIS)?
            .to_string_lossy();
        let version = egl.query_string(display, egl::VERSION)?.to_string_lossy();
        let extensions = {
            let p = egl
                .query_string(display, egl::EXTENSIONS)?
                .to_string_lossy();
            p.split(' ')
                .map(|e| e.to_string())
                .filter(|e| !e.is_empty())
                .collect::<Vec<_>>()
        };
        Ok(EglInfo::new(vender, api, version, extensions))
    }
}

impl std::fmt::Display for EglInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        writeln!(f, "EGL Info")?;
        writeln!(f, "  Vender: {}", self.vender)?;
        writeln!(f, "  API: {}", self.api)?;
        writeln!(f, "  Version: {}", self.version)?;
        write!(f, "  Extensions: {}", self.extensions.len())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_egl() {
        let context = EglContext::no_gui(&Config::default()).unwrap();
        assert!(context.display.check_x11_support());
    }
}
