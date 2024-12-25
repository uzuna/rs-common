//! JetsonのRealTimeOSTraceに関するモジュール
//!
//! sysファイル経由でトレース設定の変更ができる
use std::fs::File;
use std::io::Write;

struct TraceParam {
    trace_on: &'static str,
    trace_buffer_size: &'static str,
    trace_rtcpu: &'static str,
    trace_rtos: &'static str,
    trace_camrtc: &'static str,
    trace: &'static str,
    trace_buffersize_kb: &'static [u8; 6],
    value_lv2: &'static [u8; 2],
    value_on: &'static [u8; 2],
    value_off: &'static [u8; 2],
}

impl TraceParam {
    /// カーネルデバッグ用のパス
    pub const BASE: &'static str = "/sys/kernel/debug";

    /// デフォルトのパラメータ
    pub const DEFAULT: Self = Self {
        trace_on: "/tracing/tracing_on",
        trace_buffer_size: "/tracing/buffer_size_kb",
        trace_rtcpu: "/tracing/events/tegra_rtcpu/enable",
        trace_rtos: "/tracing/events/freertos/enable",
        trace_camrtc: "/camrtc/log-level",
        trace: "/tracing/trace",
        trace_buffersize_kb: b"30720\n",
        value_lv2: b"2\n",
        value_on: b"1\n",
        value_off: b"0\n",
    };
}

impl Default for TraceParam {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[inline]
fn file_write(path: &str, value: &[u8]) -> std::io::Result<()> {
    File::options().write(true).open(path)?.write_all(value)?;
    Ok(())
}

/// リアルタイムトレースを有効にする
pub struct Trace {
    base: String,
    param: TraceParam,
}

impl Trace {
    /// トレースを開始
    pub fn new() -> std::io::Result<Self> {
        let param = TraceParam::default();
        let s = Self {
            base: TraceParam::BASE.to_string(),
            param,
        };
        s.enable()?;
        Ok(s)
    }

    /// トレース対象のディレクトリを指定して開始
    pub fn with_base(base: impl Into<String>) -> std::io::Result<Self> {
        let param = TraceParam::default();
        let s = Self {
            base: base.into(),
            param,
        };
        s.enable()?;
        Ok(s)
    }

    /// Enable RealTimeTrace
    pub fn enable(&self) -> std::io::Result<()> {
        let param = &self.param;
        // L4T35.3.1などでは通常起動時点で1である。もしもなっていない場合のための書き込み。
        self.file_write(&param.trace_on, param.value_on)?;
        // カメラ関係のトレースの設定
        // バッファサイズを増やしてRTCPU(MIPIブロック)、RTOS(FALCON)を有効化
        // カメラトレースのログレベル設定
        self.file_write(&param.trace_buffer_size, param.trace_buffersize_kb)?;
        self.file_write(&param.trace_rtcpu, param.value_on)?;
        self.file_write(&param.trace_rtos, param.value_on)?;
        self.file_write(&param.trace_camrtc, param.value_lv2)
    }

    /// Disable RealTimeTrace
    pub fn disable(&self) -> std::io::Result<()> {
        let param = &self.param;
        // カメラ関係のトレースの設定の解除
        self.file_write(&param.trace_rtcpu, param.value_off)?;
        self.file_write(&param.trace_rtos, param.value_off)?;
        self.file_write(&param.trace_camrtc, param.value_off)
    }

    /// トレースログの中身をクリア
    pub fn clear(&self) -> std::io::Result<()> {
        // トレースの中身を空にして今からトレースするものだけにする
        self.file_write(&self.param.trace, b"")
    }

    /// 構造体は気
    pub fn forget(self) {
        std::mem::forget(self);
    }

    /// トレースログのファイルを開く
    pub fn file(&self) -> std::io::Result<File> {
        File::open(self.path(&self.param.trace))
    }

    // パスを結合
    fn path(&self, path: &str) -> String {
        format!("{}/{}", self.base, path)
    }

    #[inline]
    fn file_write(&self, path: &str, value: &[u8]) -> std::io::Result<()> {
        file_write(&self.path(path), value)
    }
}

impl Drop for Trace {
    fn drop(&mut self) {
        if let Err(e) = self.disable() {
            eprintln!("disable tracelog failed: {}", e);
        }
    }
}
