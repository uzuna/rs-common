use anyhow::{anyhow, bail};
use wasmtime::{Config, Engine, ProfilingStrategy};

pub const WASMTIME_PROFILER_ENV: &str = "MOONBIT_RUNNER_WASMTIME_PROFILER";

pub fn create_engine_from_env() -> anyhow::Result<Engine> {
    let profiler = match std::env::var(WASMTIME_PROFILER_ENV) {
        Ok(raw) => Some(parse_profiling_strategy(&raw)?),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => {
            bail!("{WASMTIME_PROFILER_ENV} must be valid UTF-8")
        }
    };

    if let Some(profiler) = profiler {
        let mut config = Config::new();
        config.profiler(profiler);
        Engine::new(&config).map_err(|err| {
            anyhow!(
                "failed to create Wasmtime engine with {} enabled: {err}",
                WASMTIME_PROFILER_ENV
            )
        })
    } else {
        Ok(Engine::default())
    }
}

fn parse_profiling_strategy(value: &str) -> anyhow::Result<ProfilingStrategy> {
    let normalized = value.trim().to_ascii_lowercase();
    let strategy = match normalized.as_str() {
        "none" => ProfilingStrategy::None,
        "perfmap" | "perf_map" | "perf-map" => ProfilingStrategy::PerfMap,
        "jitdump" | "jit_dump" | "jit-dump" => ProfilingStrategy::JitDump,
        "vtune" => ProfilingStrategy::VTune,
        "pulley" => ProfilingStrategy::Pulley,
        _ => {
            bail!(
                "invalid {WASMTIME_PROFILER_ENV}={value}; expected one of: none, perfmap, jitdump, vtune, pulley"
            )
        }
    };
    Ok(strategy)
}
