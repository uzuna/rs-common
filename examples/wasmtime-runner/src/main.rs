use anyhow::Context;
use clap::Parser;
use context::{run_hasdep, run_sequence_hello, WasmComponent};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use wasmtime::*;

pub mod bingings;
pub mod context;

#[derive(Debug, Clone, clap::Parser)]
struct Opt {
    #[arg(long, default_value = "config.yaml")]
    pub config: PathBuf,
}

/// WASIサポートの種類を表す列挙型
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WasiSupport {
    #[default]
    None,
    Preview2,
}

/// プラグインのがサポートしているインターフェースの種類
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Plugin {
    Hello,
    Hasdep,
}

/// プラグインに対応する機能を記載する構造体
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PluginPair {
    /// プラグインの名前
    pub plugin: Vec<Plugin>,
    /// 必要なWASIサポートの種類
    #[serde(default)]
    pub wasi: WasiSupport,
    /// プラグインのバイナリファイル名
    pub file: PathBuf,
}

impl PluginPair {
    pub fn join_base(&self, base: &Path) -> Self {
        let mut new_pair = self.clone();
        new_pair.file = base.join(&self.file);
        new_pair
    }
}

/// このバイナリが読み出すプラグインの設定記述向けパース構造体
///
/// 記述しやすさが有線で実際の保持型は別途作っても良い
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginConfig {
    /// プラグインのベースパス
    pub dir: PathBuf,
    /// プラグインとバイナリファイル名のマッピング
    pub pairs: Vec<PluginPair>,
}

fn run_inner<T>(c: &mut WasmComponent<T>, p: Plugin) -> anyhow::Result<()> {
    match p {
        Plugin::Hello => run_sequence_hello(c),
        Plugin::Hasdep => run_hasdep(c),
    }
}

fn run(engine: &Engine, pair: &PluginPair) -> anyhow::Result<()> {
    let buffer = std::fs::read(&pair.file)
        .with_context(|| format!("Failed to read wasm file: {}", pair.file.display()))?;

    match pair.wasi {
        WasiSupport::None => {
            println!("Running without WASI support for plugin {:?}", pair.plugin);
            let mut c = WasmComponent::new_unknown(engine, buffer.as_slice(), ())?;
            for p in &pair.plugin {
                run_inner(&mut c, *p)?;
            }
        }
        WasiSupport::Preview2 => {
            println!(
                "Running with WASI Preview2 support for plugin {:?}",
                pair.plugin
            );
            let mut c = WasmComponent::new_p2(engine, buffer.as_slice())?;
            for p in &pair.plugin {
                run_inner(&mut c, *p)?;
            }
        }
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let opt = Opt::parse();
    let config_path = opt.config;
    let config: PluginConfig = serde_yaml::from_reader(
        std::fs::File::open(&config_path)
            .with_context(|| format!("Failed to open config file: {}", config_path.display()))?,
    )
    .with_context(|| format!("Failed to parse config file: {}", config_path.display()))?;

    let engine = Engine::default();
    for pair in &config.pairs {
        let pair = pair.join_base(&config.dir);
        // 各プラグインの実行
        run(&engine, &pair).with_context(|| {
            format!(
                "Failed to run plugin: {:?} with file: {}",
                pair.plugin,
                pair.file.display()
            )
        })?;
    }
    Ok(())
}
