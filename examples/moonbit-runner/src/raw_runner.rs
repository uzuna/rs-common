//! plain core Wasm に対して線形メモリを直接操作する benchmark を行う

use anyhow::{anyhow, ensure, Context};
use std::path::Path;
use std::time::Instant;
use wasmtime::{Engine, Instance, Linker, Memory, Module, Store, TypedFunc};

use crate::runner::{SizeBenchmarkReport, SizeBenchmarkResult};

const MEMORY_EXPORT: &str = "memory";
const PAGE_SIZE_BYTES: usize = 65_536;
const BENCHMARK_RAW_128_EXPORT: &str = "benchmark_raw_128";
const BENCHMARK_RAW_1K_EXPORT: &str = "benchmark_raw_1k";
const BENCHMARK_RAW_4K_EXPORT: &str = "benchmark_raw_4k";

pub fn run_size_benchmarks(path: &Path, iterations: usize) -> anyhow::Result<SizeBenchmarkReport> {
    let mut module = RawBenchmarkModule::new(path)?;
    Ok(SizeBenchmarkReport {
        b128: module.benchmark(BENCHMARK_RAW_128_EXPORT, 128, iterations)?,
        b1k: module.benchmark(BENCHMARK_RAW_1K_EXPORT, 1024, iterations)?,
        b4k: module.benchmark(BENCHMARK_RAW_4K_EXPORT, 4096, iterations)?,
    })
}

struct RawBenchmarkModule {
    store: Store<()>,
    memory: Memory,
    instance: Instance,
    base_offset: usize,
}

impl RawBenchmarkModule {
    fn new(path: &Path) -> anyhow::Result<Self> {
        let engine = Engine::default();
        let module = Module::from_file(&engine, path)
            .map_err(|err| anyhow!("plain Wasm の読み込み失敗: {}: {err}", path.display()))?;
        let linker = Linker::new(&engine);
        let mut store = Store::new(&engine, ());
        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|err| anyhow!("plain Wasm インスタンス化に失敗しました: {err}"))?;
        let memory = instance
            .get_memory(&mut store, MEMORY_EXPORT)
            .context("plain Wasm に memory export がありません")?;
        let current_pages = memory.size(&store);
        memory
            .grow(&mut store, 1)
            .map_err(|err| anyhow!("benchmark 用メモリページの拡張に失敗しました: {err}"))?;
        let base_offset = (current_pages as usize) * PAGE_SIZE_BYTES;
        Ok(Self {
            store,
            memory,
            instance,
            base_offset,
        })
    }

    fn benchmark(
        &mut self,
        call_export: &str,
        payload_bytes: usize,
        iterations: usize,
    ) -> anyhow::Result<SizeBenchmarkResult> {
        let call = self
            .instance
            .get_typed_func::<(i32, i32), i32>(&mut self.store, call_export)
            .map_err(|err| {
                anyhow!("benchmark export の解決に失敗しました: {call_export}: {err}")
            })?;
        let input_ptr = self.base_offset as i32;
        ensure!(
            self.memory.data_size(&self.store) >= self.base_offset + payload_bytes,
            "benchmark 用メモリ領域が不足しています: need={} actual={}",
            self.base_offset + payload_bytes,
            self.memory.data_size(&self.store)
        );

        let mut payload = build_payload(payload_bytes);
        let mut output = vec![0u8; payload_bytes];
        self.run_probe(&call, input_ptr, &payload, &mut output)?;

        let mut max_ns: u64 = 0;
        let total_start = Instant::now();
        for iteration in 0..iterations {
            update_payload(&mut payload, iteration);
            let call_start = Instant::now();
            self.roundtrip(&call, input_ptr, &payload, &mut output)?;
            let ns = call_start.elapsed().as_nanos() as u64;
            if ns > max_ns {
                max_ns = ns;
            }
        }
        let elapsed = total_start.elapsed();

        Ok(SizeBenchmarkResult {
            payload_bytes,
            iterations,
            elapsed,
            avg_ns: if iterations == 0 {
                0
            } else {
                (elapsed.as_nanos() as u64) / (iterations as u64)
            },
            max_ns,
            pps: if elapsed.is_zero() {
                0.0
            } else {
                iterations as f64 / elapsed.as_secs_f64()
            },
        })
    }

    fn run_probe(
        &mut self,
        call: &TypedFunc<(i32, i32), i32>,
        input_ptr: i32,
        payload: &[u8],
        output: &mut [u8],
    ) -> anyhow::Result<()> {
        self.roundtrip(call, input_ptr, payload, output)?;
        ensure!(
            output == payload,
            "raw Wasm の roundtrip 結果が入力と一致しません"
        );
        Ok(())
    }

    fn roundtrip(
        &mut self,
        call: &TypedFunc<(i32, i32), i32>,
        input_ptr: i32,
        payload: &[u8],
        output: &mut [u8],
    ) -> anyhow::Result<()> {
        self.memory
            .write(&mut self.store, input_ptr as usize, payload)
            .context("input を Wasm 線形メモリへ書き込めませんでした")?;
        let out_len = call
            .call(&mut self.store, (input_ptr, payload.len() as i32))
            .map_err(|err| anyhow!("plain Wasm benchmark 呼び出しに失敗しました: {err}"))?;
        ensure!(
            out_len == payload.len() as i32,
            "raw Wasm の出力長が想定外です: expected={} actual={out_len}",
            payload.len()
        );
        self.memory
            .read(
                &self.store,
                input_ptr as usize,
                &mut output[..out_len as usize],
            )
            .context("output を Wasm 線形メモリから読み出せませんでした")?;
        Ok(())
    }
}

fn build_payload(payload_bytes: usize) -> Vec<u8> {
    (0..payload_bytes)
        .map(|index| ((index * 31 + 17) % 251) as u8)
        .collect()
}

fn update_payload(payload: &mut [u8], iteration: usize) {
    if payload.is_empty() {
        return;
    }
    let head = (iteration % 251) as u8;
    let tail = ((iteration * 3) % 251) as u8;
    payload[0] = head;
    payload[payload.len() - 1] = tail;
}
