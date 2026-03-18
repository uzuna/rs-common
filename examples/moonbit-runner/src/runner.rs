//! MoonBitプラグインの実行手順とベンチマークをまとめる

use anyhow::{bail, ensure, Context};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use wasmtime::{component::Component, Engine};

use crate::bindings::{MotorOutput, PluginInst, PluginStatus, SensorData};
use crate::context::ExecStore;

const FLOAT_EPSILON: f32 = 1.0e-6;
const PROBE_VALID_POSITION: f32 = 30.0;
const PROBE_VALID_TORQUE: f32 = 10.0;

/// 利用するWASIサポートの種類
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum WasiSupport {
    #[default]
    None,
    Preview2,
}

impl WasiSupport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Preview2 => "preview2",
        }
    }
}

/// ランナーの設定値
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerConfig {
    /// 読み込むWasmコンポーネントのパス
    pub wasm: PathBuf,
    /// 利用を要求するWASIサポート
    pub wasi: WasiSupport,
    /// PPS計測時の `update` 呼び出し回数
    pub benchmark_iterations: usize,
}

impl RunnerConfig {
    /// 実行前に最低限の設定値を検証する
    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            self.benchmark_iterations > 0,
            "ベンチマーク回数は1以上である必要があります"
        );
        Ok(())
    }
}

/// ベンチマーク計測結果
#[derive(Debug, Clone)]
pub struct BenchmarkResult {
    /// `update` を呼び出した回数
    pub iterations: usize,
    /// 計測にかかった総時間
    pub elapsed: Duration,
    /// 1秒あたりの処理回数
    pub pps: f64,
}

impl BenchmarkResult {
    fn from_elapsed(iterations: usize, elapsed: Duration) -> Self {
        let pps = if elapsed.is_zero() {
            0.0
        } else {
            iterations as f64 / elapsed.as_secs_f64()
        };
        Self {
            iterations,
            elapsed,
            pps,
        }
    }
}

/// 実行結果のサマリ
#[derive(Debug, Clone)]
pub struct RunReport {
    /// 実行開始時のステータス
    pub initial_status: PluginStatus,
    /// 検証用入力に対する出力
    pub probe_outputs: Vec<MotorOutput>,
    /// 検証用入力実行後のステータス
    pub probe_status: PluginStatus,
    /// PPSベンチマーク結果
    pub benchmark: BenchmarkResult,
    /// ベンチマーク後の最終ステータス
    pub final_status: PluginStatus,
}

/// Phase3 で必要な単発検証と PPS ベンチマークを順に実行する
pub fn run(config: &RunnerConfig) -> anyhow::Result<RunReport> {
    config.validate()?;
    ensure_supported_wasi(config.wasi)?;

    let engine = Engine::default();
    let component = load_component(&engine, &config.wasm)?;
    let mut inst = instantiate_plugin(&engine, &component, config.wasi)?;

    let initial_status = inst
        .get_status()
        .context("初期ステータスの取得に失敗しました")?;

    let probe_input = build_probe_input();
    let probe_outputs = inst
        .update(&probe_input)
        .context("検証用の update 呼び出しに失敗しました")?;
    let probe_status = inst
        .get_status()
        .context("検証後ステータスの取得に失敗しました")?;
    verify_probe_result(&probe_outputs, &probe_status)?;

    let benchmark_input = build_benchmark_input();
    let benchmark = benchmark_updates(&mut inst, config.benchmark_iterations, &benchmark_input)?;
    let final_status = inst
        .get_status()
        .context("ベンチマーク後ステータスの取得に失敗しました")?;

    Ok(RunReport {
        initial_status,
        probe_outputs,
        probe_status,
        benchmark,
        final_status,
    })
}

/// 実行結果を標準出力へ整形して表示する
pub fn print_report(config: &RunnerConfig, report: &RunReport) {
    println!(
        "実行設定: wasm={}, wasi={}, iterations={}",
        config.wasm.display(),
        config.wasi.as_str(),
        config.benchmark_iterations
    );
    println!(
        "初期ステータス: running={}, error_code={}, temperature={}",
        report.initial_status.running,
        report.initial_status.error_code,
        report.initial_status.temperature
    );
    for (index, output) in report.probe_outputs.iter().enumerate() {
        println!(
            "検証出力[{index}]: position={}, torque={}",
            output.position, output.torque
        );
    }
    println!(
        "検証後ステータス: running={}, error_code={}, temperature={}",
        report.probe_status.running,
        report.probe_status.error_code,
        report.probe_status.temperature
    );
    println!(
        "PPSベンチマーク: iterations={}, elapsed_ms={:.3}, pps={:.2}",
        report.benchmark.iterations,
        report.benchmark.elapsed.as_secs_f64() * 1_000.0,
        report.benchmark.pps
    );
    println!(
        "最終ステータス: running={}, error_code={}, temperature={}",
        report.final_status.running,
        report.final_status.error_code,
        report.final_status.temperature
    );
}

fn ensure_supported_wasi(wasi: WasiSupport) -> anyhow::Result<()> {
    match wasi {
        WasiSupport::None => Ok(()),
        WasiSupport::Preview2 => {
            bail!("WASI Preview2 は moonbit-runner では未対応です")
        }
    }
}

fn load_component(engine: &Engine, path: &Path) -> anyhow::Result<Component> {
    let buffer = std::fs::read(path)
        .with_context(|| format!("Wasmファイルの読み込み失敗: {}", path.display()))?;
    Component::new(engine, &buffer)
        .map_err(|e| anyhow::anyhow!("Componentの生成失敗: {}: {}", path.display(), e))
}

fn instantiate_plugin(
    engine: &Engine,
    component: &Component,
    wasi: WasiSupport,
) -> anyhow::Result<PluginInst> {
    match wasi {
        WasiSupport::None => {
            let store = ExecStore::new(engine);
            PluginInst::new_with_binary(store, component)
                .context("WASIなしプラグインの初期化に失敗しました")
        }
        WasiSupport::Preview2 => {
            bail!("WASI Preview2 は moonbit-runner では未対応です")
        }
    }
}

fn build_probe_input() -> Vec<SensorData> {
    vec![
        SensorData {
            load: PROBE_VALID_TORQUE,
            position: PROBE_VALID_POSITION,
            extra: Some(1.0),
        },
        SensorData {
            load: 200.0,
            position: 999.0,
            extra: None,
        },
    ]
}

fn build_benchmark_input() -> Vec<SensorData> {
    (0..64)
        .map(|index| SensorData {
            load: (index as f32) * 0.5,
            position: index as f32 - 32.0,
            extra: if index % 2 == 0 {
                Some(index as f32 * 0.25)
            } else {
                None
            },
        })
        .collect()
}

fn benchmark_updates(
    inst: &mut PluginInst,
    iterations: usize,
    input: &[SensorData],
) -> anyhow::Result<BenchmarkResult> {
    let started = Instant::now();
    for _ in 0..iterations {
        inst.update(input)
            .context("PPSベンチマーク中の update 呼び出しに失敗しました")?;
    }
    Ok(BenchmarkResult::from_elapsed(iterations, started.elapsed()))
}

fn verify_probe_result(outputs: &[MotorOutput], status: &PluginStatus) -> anyhow::Result<()> {
    ensure!(
        outputs.len() == 2,
        "検証出力数が想定外です: expected=2 actual={}",
        outputs.len()
    );

    let first = &outputs[0];
    ensure!(
        approx_eq(first.position, PROBE_VALID_POSITION),
        "正常出力の位置が想定外です: actual={}",
        first.position
    );
    ensure!(
        approx_eq(first.torque, PROBE_VALID_TORQUE),
        "正常出力のトルクが想定外です: actual={}",
        first.torque
    );

    let second = &outputs[1];
    ensure!(
        approx_eq(second.position, 0.0),
        "安全側出力の位置が想定外です: actual={}",
        second.position
    );
    ensure!(
        approx_eq(second.torque, 0.0),
        "安全側出力のトルクが想定外です: actual={}",
        second.torque
    );

    ensure!(
        !status.running,
        "検証後ステータスが停止状態になっていません"
    );
    ensure!(
        status.error_code == 1,
        "検証後エラーコードが想定外です: actual={}",
        status.error_code
    );
    ensure!(
        status.temperature.is_finite(),
        "検証後温度が有限値ではありません: actual={}",
        status.temperature
    );
    Ok(())
}

fn approx_eq(left: f32, right: f32) -> bool {
    (left - right).abs() <= FLOAT_EPSILON
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ConfigCase {
        name: &'static str,
        iterations: usize,
        expect_ok: bool,
        expected_message: &'static str,
    }

    struct ProbeCase {
        name: &'static str,
        outputs: Vec<MotorOutput>,
        status: PluginStatus,
        expect_ok: bool,
        expected_message: &'static str,
    }

    fn assert_config_case(case: ConfigCase) {
        let config = RunnerConfig {
            wasm: PathBuf::from("plugins/control.component.wasm"),
            wasi: WasiSupport::None,
            benchmark_iterations: case.iterations,
        };
        let result = config.validate();
        assert_eq!(
            result.is_ok(),
            case.expect_ok,
            "設定検証ケース `{}` の成否が想定と異なります",
            case.name
        );
        if let Err(err) = result {
            assert!(
                err.to_string().contains(case.expected_message),
                "設定検証ケース `{}` のエラー内容が想定と異なります: {}",
                case.name,
                err
            );
        }
    }

    fn assert_probe_case(case: ProbeCase) {
        let result = verify_probe_result(&case.outputs, &case.status);
        assert_eq!(
            result.is_ok(),
            case.expect_ok,
            "プローブ検証ケース `{}` の成否が想定と異なります",
            case.name
        );
        if let Err(err) = result {
            assert!(
                err.to_string().contains(case.expected_message),
                "プローブ検証ケース `{}` のエラー内容が想定と異なります: {}",
                case.name,
                err
            );
        }
    }

    #[test]
    fn 設定検証_値域確認() {
        let cases = [
            ConfigCase {
                name: "最小有効回数",
                iterations: 1,
                expect_ok: true,
                expected_message: "",
            },
            ConfigCase {
                name: "通常回数",
                iterations: 10_000,
                expect_ok: true,
                expected_message: "",
            },
            ConfigCase {
                name: "ゼロ回は不可",
                iterations: 0,
                expect_ok: false,
                expected_message: "ベンチマーク回数は1以上",
            },
        ];

        for case in cases {
            assert_config_case(case);
        }
    }

    #[test]
    fn プローブ検証_正常系() {
        let cases = [ProbeCase {
            name: "正常出力と異常検知が混在する想定ケース",
            outputs: vec![
                MotorOutput {
                    position: PROBE_VALID_POSITION,
                    torque: PROBE_VALID_TORQUE,
                },
                MotorOutput {
                    position: 0.0,
                    torque: 0.0,
                },
            ],
            status: PluginStatus {
                running: false,
                error_code: 1,
                temperature: 0.0,
            },
            expect_ok: true,
            expected_message: "",
        }];

        for case in cases {
            assert_probe_case(case);
        }
    }

    #[test]
    fn プローブ検証_異常系() {
        let cases = [
            ProbeCase {
                name: "出力数不足",
                outputs: vec![MotorOutput {
                    position: PROBE_VALID_POSITION,
                    torque: PROBE_VALID_TORQUE,
                }],
                status: PluginStatus {
                    running: false,
                    error_code: 1,
                    temperature: 0.0,
                },
                expect_ok: false,
                expected_message: "検証出力数が想定外",
            },
            ProbeCase {
                name: "安全側出力のトルク不一致",
                outputs: vec![
                    MotorOutput {
                        position: PROBE_VALID_POSITION,
                        torque: PROBE_VALID_TORQUE,
                    },
                    MotorOutput {
                        position: 0.0,
                        torque: 0.5,
                    },
                ],
                status: PluginStatus {
                    running: false,
                    error_code: 1,
                    temperature: 0.0,
                },
                expect_ok: false,
                expected_message: "安全側出力のトルクが想定外",
            },
            ProbeCase {
                name: "エラーコード未反映",
                outputs: vec![
                    MotorOutput {
                        position: PROBE_VALID_POSITION,
                        torque: PROBE_VALID_TORQUE,
                    },
                    MotorOutput {
                        position: 0.0,
                        torque: 0.0,
                    },
                ],
                status: PluginStatus {
                    running: true,
                    error_code: 0,
                    temperature: 0.0,
                },
                expect_ok: false,
                expected_message: "検証後ステータスが停止状態になっていません",
            },
        ];

        for case in cases {
            assert_probe_case(case);
        }
    }
}
