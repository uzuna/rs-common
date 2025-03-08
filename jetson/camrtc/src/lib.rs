//! JetsonのRealTimeOSTraceに関するモジュール
//!
//! sysファイル経由でトレース設定の変更ができる
use std::io::{self, Write};
use std::path::Path;

use fs_err::{self as fs, File};

// トレース出力操作対象
#[derive(Clone, Copy)]
enum Target {
    // トレースの有効or無効。無効化するとハングアップするので注意(L4T 35.3.1)
    TracingOn,
    // トレースバッファサイズ(KB単位)。これ以上になるt古いものから上書きされる
    BufferSize,
    // RTCPUトレースの有効or無効。おそらくISP関連
    RtCpuEnable,
    // FreeRTOSトレースの有効or無効。おそらくFALCON(MIPI-CSI管理ブロック)関連
    FreeRtosEnable,
    // カメラRTCトレースのログ出力レベル
    CamRtcLogLevel,
    // 取得したトレースログにアクセスするためのファイル
    Trace,
}

// 書き込む値
#[derive(Clone, Copy)]
enum Value {
    On,
    Off,
    LogLevel2,
    DefaultBufferSize,
}

impl Value {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::On => b"1\n",
            Self::Off => b"0\n",
            // このあたりのデフォルト値はNVIDIA ForumやRidgeRunが提供するドキュメントに基づいている
            // ref: https://elinux.org/Jetson/l4t/Camera_BringUp
            Self::LogLevel2 => b"2\n",
            Self::DefaultBufferSize => b"30720\n",
        }
    }
}

#[derive(Clone, Copy)]
struct Paths<'a> {
    trace_on: &'a Path,
    trace_buffer_size: &'a Path,
    trace_rtcpu: &'a Path,
    trace_rtos: &'a Path,
    trace_camrtc: &'a Path,
    trace: &'a Path,
}

impl<'a> Paths<'a> {
    /// 実パスを取得
    fn target(&self, target: Target) -> &'a Path {
        match target {
            Target::TracingOn => self.trace_on,
            Target::BufferSize => self.trace_buffer_size,
            Target::RtCpuEnable => self.trace_rtcpu,
            Target::FreeRtosEnable => self.trace_rtos,
            Target::CamRtcLogLevel => self.trace_camrtc,
            Target::Trace => self.trace,
        }
    }
}

impl Default for Paths<'static> {
    fn default() -> Self {
        Paths {
            trace_on: Path::new("/sys/kernel/debug/tracing/tracing_on"),
            trace_buffer_size: Path::new("/sys/kernel/debug/tracing/buffer_size_kb"),
            trace_rtcpu: Path::new("/sys/kernel/debug/tracing/events/tegra_rtcpu/enable"),
            trace_rtos: Path::new("/sys/kernel/debug/tracing/events/freertos/enable"),
            trace_camrtc: Path::new("/sys/kernel/debug/camrtc/log-level"),
            trace: Path::new("/sys/kernel/debug/tracing/trace"),
        }
    }
}

#[inline]
fn file_write(path: &Path, value: &[u8]) -> io::Result<()> {
    let file = std::fs::File::options().write(true).open(path)?;
    File::from_parts(file, path).write_all(value)?;
    Ok(())
}

// トレースログをclearするための書き込み
#[inline]
fn file_clear_write(path: &Path, value: &[u8]) -> io::Result<()> {
    fs::write(path, value)
}

/// リアルタイムトレースを有効にする
///
/// 動作させるのにroot権限が必要です(カメラトレース関係のファイルのgroup:ownerがrootであるため)
pub struct Trace<'a> {
    paths: Paths<'a>,
}

impl Trace<'static> {
    /// トレースを開始
    pub fn new() -> io::Result<Self> {
        Self::with_paths(Paths::default())
    }
}

impl<'a> Trace<'a> {
    fn with_paths(paths: Paths<'a>) -> io::Result<Self> {
        let this = Self { paths };
        this.enable()?;
        Ok(this)
    }

    const ENABLES: &'static [(Target, Value)] = &[
        // L4T35.3.1などでは通常起動時点で1である。もしもなっていない場合のための書き込み。
        (Target::TracingOn, Value::On),
        // カメラ関係のトレースの設定
        // バッファサイズを増やしてRTCPU(MIPIブロック)、RTOS(FALCON)を有効化
        // カメラトレースのログレベル設定
        (Target::BufferSize, Value::DefaultBufferSize),
        (Target::RtCpuEnable, Value::On),
        (Target::FreeRtosEnable, Value::On),
        (Target::CamRtcLogLevel, Value::LogLevel2),
    ];

    // カメラ関係のトレースの設定の解除
    const DISABLES: &'static [(Target, Value)] = &[
        (Target::RtCpuEnable, Value::Off),
        (Target::FreeRtosEnable, Value::Off),
        (Target::CamRtcLogLevel, Value::Off),
    ];

    fn path(&self, target: Target) -> &'a Path {
        self.paths.target(target)
    }

    /// トレース有効化
    pub fn enable(&self) -> io::Result<()> {
        for &(target, value) in Self::ENABLES {
            file_write(self.path(target), value.as_bytes())?;
        }
        Ok(())
    }

    /// トレース無効化
    pub fn disable(&self) -> io::Result<()> {
        for &(target, value) in Self::DISABLES {
            file_write(self.path(target), value.as_bytes())?;
        }
        Ok(())
    }

    /// トレースログの中身をクリア
    pub fn clear(&self) -> io::Result<()> {
        // トレースの中身を空にして今からトレースするものだけにする
        file_clear_write(self.path(Target::Trace), b"")
    }

    /// トレース無効化をしない
    pub fn forget(self) {
        std::mem::forget(self);
    }

    /// トレースログのファイルを開く
    pub fn file(&self) -> io::Result<File> {
        File::open(self.path(Target::Trace))
    }
}

impl Drop for Trace<'_> {
    fn drop(&mut self) {
        if let Err(e) = self.disable() {
            tracing::error!("disable tracelog failed: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_behavior() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path();
        // ふるまい確認用のダミーパラメータ
        let dummy_paths = Paths {
            trace_on: &path.join("tracing_on"),
            trace_buffer_size: &path.join("buffer_size_kb"),
            trace_rtcpu: &path.join("tegra_rtcpu"),
            trace_rtos: &path.join("freertos"),
            trace_camrtc: &path.join("camrtc"),
            trace: &path.join("trace"),
        };

        // ファイルがないときは失敗する
        let res = Trace::with_paths(dummy_paths);
        assert!(res.is_err());

        // OS起動時にファイルがあるのが気が期待動作なので作成
        for &(target, _) in Trace::ENABLES.iter().chain(Trace::DISABLES) {
            File::create(dummy_paths.target(target))?;
        }

        // enable変更
        let trace = Trace::with_paths(dummy_paths)?;
        for &(target, value) in Trace::ENABLES {
            let content = std::fs::read(dummy_paths.target(target))?;
            assert_eq!(content, value.as_bytes());
        }

        // 書き込みを模擬して、中身がクリアできることを確認
        std::fs::write(dummy_paths.target(Target::Trace), b"dummy")?;
        trace.clear()?;
        let content = std::fs::read(dummy_paths.target(Target::Trace))?;
        assert_eq!(content, b"");

        // dropでdisable
        drop(trace);
        for &(target, value) in Trace::DISABLES {
            let content = std::fs::read(dummy_paths.target(target))?;
            assert_eq!(content, value.as_bytes());
        }

        // forgetしたらdisableされない
        let trace = Trace::with_paths(dummy_paths)?;
        trace.forget();
        for &(target, value) in Trace::ENABLES {
            let content = std::fs::read(dummy_paths.target(target))?;
            assert_eq!(content, value.as_bytes());
        }
        Ok(())
    }
}
