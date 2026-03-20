use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use moonbit_runner::bindings::{
    BenchmarkInput128, BenchmarkInput1k, BenchmarkInput4k, PluginInst, SensorData,
};
use moonbit_runner::context::ExecStore;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use wasmtime::{component::Component, Engine, Instance, Linker, Memory, Module, Store, TypedFunc};

const PAGE_SIZE_BYTES: usize = 65_536;
const RAW_EXPORT_128: &str = "benchmark_raw_128";
const RAW_EXPORT_1K: &str = "benchmark_raw_1k";
const RAW_EXPORT_4K: &str = "benchmark_raw_4k";
const RAW_EXPORT_ADD: &str = "add_raw";
const ADD_RAW_TRANSFER_BYTES: usize = 16;

fn plugin_path(file_name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("plugins")
        .join(file_name)
}

struct WitHarness {
    inst: PluginInst,
    update_input: Vec<SensorData>,
    in128: BenchmarkInput128,
    in1k: BenchmarkInput1k,
    in4k: BenchmarkInput4k,
}

impl WitHarness {
    fn new(component_path: &Path) -> Self {
        let engine = Engine::default();
        let bytes = std::fs::read(component_path).unwrap_or_else(|err| {
            panic!(
                "component Wasm の読み込みに失敗しました: {}: {err}",
                component_path.display()
            )
        });
        let component = Component::new(&engine, &bytes).unwrap_or_else(|err| {
            panic!(
                "component Wasm のロードに失敗しました: {}: {err}",
                component_path.display()
            )
        });
        let store = ExecStore::new(&engine);
        let inst = PluginInst::new_with_binary(store, &component)
            .expect("component プラグインの初期化に失敗しました");

        Self {
            inst,
            update_input: build_update_input(),
            in128: BenchmarkInput128 {
                payload: build_payload(128),
            },
            in1k: BenchmarkInput1k {
                payload: build_payload(1024),
            },
            in4k: BenchmarkInput4k {
                payload: build_payload(4096),
            },
        }
    }

    fn bench_update_once(&mut self) {
        let output = self
            .inst
            .update(&self.update_input)
            .expect("update ベンチマーク呼び出しに失敗しました");
        black_box(output);
    }

    fn bench_128_once(&mut self) {
        let output = self
            .inst
            .benchmark_128(&self.in128)
            .expect("benchmark_128 ベンチマーク呼び出しに失敗しました");
        black_box(output.payload.len());
    }

    fn bench_1k_once(&mut self) {
        let output = self
            .inst
            .benchmark_1k(&self.in1k)
            .expect("benchmark_1k ベンチマーク呼び出しに失敗しました");
        black_box(output.payload.len());
    }

    fn bench_4k_once(&mut self) {
        let output = self
            .inst
            .benchmark_4k(&self.in4k)
            .expect("benchmark_4k ベンチマーク呼び出しに失敗しました");
        black_box(output.payload.len());
    }

    fn bench_add_loop1_once(&mut self) {
        let value = self
            .inst
            .add(11, 7, 1)
            .expect("add(loop=1) ベンチマーク呼び出しに失敗しました");
        black_box(value);
    }

    fn bench_add_heavy_once(&mut self) {
        let value = self
            .inst
            .add(11, 7, 2000)
            .expect("add(loop=2000) ベンチマーク呼び出しに失敗しました");
        black_box(value);
    }
}

struct RawHarness {
    store: Store<()>,
    memory: Memory,
    ptr: i32,
    tick: u64,
    call_128: TypedFunc<(i32, i32), i32>,
    call_1k: TypedFunc<(i32, i32), i32>,
    call_4k: TypedFunc<(i32, i32), i32>,
    call_add: TypedFunc<(i32, i32), i32>,
    payload_128: Vec<u8>,
    payload_1k: Vec<u8>,
    payload_4k: Vec<u8>,
    output_128: Vec<u8>,
    output_1k: Vec<u8>,
    output_4k: Vec<u8>,
    add_transfer: [u8; ADD_RAW_TRANSFER_BYTES],
}

struct NativeHarness;

impl NativeHarness {
    fn new() -> Self {
        Self
    }

    fn bench_add_loop1_once(&mut self) {
        let result = add_native(black_box(11), black_box(7), black_box(1));
        black_box(result);
    }

    fn bench_add_loop2000_once(&mut self) {
        let result = add_native(black_box(11), black_box(7), black_box(2000));
        black_box(result);
    }
}

impl RawHarness {
    fn new(raw_wasm_path: &Path) -> Self {
        let engine = Engine::default();
        let module = Module::from_file(&engine, raw_wasm_path).unwrap_or_else(|err| {
            panic!(
                "raw Wasm の読み込みに失敗しました: {}: {err}",
                raw_wasm_path.display()
            )
        });
        let linker = Linker::new(&engine);
        let mut store = Store::new(&engine, ());
        let instance: Instance = linker
            .instantiate(&mut store, &module)
            .expect("raw Wasm のインスタンス化に失敗しました");
        let memory = instance
            .get_memory(&mut store, "memory")
            .expect("raw Wasm に memory export がありません");

        let current_pages = memory.size(&store);
        memory
            .grow(&mut store, 1)
            .expect("raw benchmark 用 memory.grow に失敗しました");
        let ptr = (current_pages as usize * PAGE_SIZE_BYTES) as i32;

        let call_128 = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, RAW_EXPORT_128)
            .expect("benchmark_raw_128 export がありません");
        let call_1k = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, RAW_EXPORT_1K)
            .expect("benchmark_raw_1k export がありません");
        let call_4k = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, RAW_EXPORT_4K)
            .expect("benchmark_raw_4k export がありません");
        let call_add = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, RAW_EXPORT_ADD)
            .expect("add_raw export がありません");

        Self {
            store,
            memory,
            ptr,
            tick: 0,
            call_128,
            call_1k,
            call_4k,
            call_add,
            payload_128: build_payload(128),
            payload_1k: build_payload(1024),
            payload_4k: build_payload(4096),
            output_128: vec![0u8; 128],
            output_1k: vec![0u8; 1024],
            output_4k: vec![0u8; 4096],
            add_transfer: [0u8; ADD_RAW_TRANSFER_BYTES],
        }
    }

    fn bench_128_once(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        mutate_payload(&mut self.payload_128, self.tick);
        let len = self.payload_128.len();
        self.memory
            .write(&mut self.store, self.ptr as usize, &self.payload_128)
            .expect("raw 128B: memory write に失敗しました");
        let out_len = self
            .call_128
            .call(&mut self.store, (self.ptr, len as i32))
            .expect("raw 128B: 関数呼び出しに失敗しました");
        assert_eq!(out_len as usize, len, "raw 128B: output length mismatch");
        self.memory
            .read(&self.store, self.ptr as usize, &mut self.output_128[..len])
            .expect("raw 128B: memory read に失敗しました");
        black_box(self.output_128[0]);
    }

    fn bench_1k_once(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        mutate_payload(&mut self.payload_1k, self.tick);
        let len = self.payload_1k.len();
        self.memory
            .write(&mut self.store, self.ptr as usize, &self.payload_1k)
            .expect("raw 1KB: memory write に失敗しました");
        let out_len = self
            .call_1k
            .call(&mut self.store, (self.ptr, len as i32))
            .expect("raw 1KB: 関数呼び出しに失敗しました");
        assert_eq!(out_len as usize, len, "raw 1KB: output length mismatch");
        self.memory
            .read(&self.store, self.ptr as usize, &mut self.output_1k[..len])
            .expect("raw 1KB: memory read に失敗しました");
        black_box(self.output_1k[0]);
    }

    fn bench_4k_once(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        mutate_payload(&mut self.payload_4k, self.tick);
        let len = self.payload_4k.len();
        self.memory
            .write(&mut self.store, self.ptr as usize, &self.payload_4k)
            .expect("raw 4KB: memory write に失敗しました");
        let out_len = self
            .call_4k
            .call(&mut self.store, (self.ptr, len as i32))
            .expect("raw 4KB: 関数呼び出しに失敗しました");
        assert_eq!(out_len as usize, len, "raw 4KB: output length mismatch");
        self.memory
            .read(&self.store, self.ptr as usize, &mut self.output_4k[..len])
            .expect("raw 4KB: memory read に失敗しました");
        black_box(self.output_4k[0]);
    }

    fn bench_add_raw_loop1_once(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        let a = (self.tick % 1024) as i32;
        let b = ((self.tick * 3) % 1024) as i32;
        let loop_count = 1i32;
        self.add_transfer[0..4].copy_from_slice(&a.to_le_bytes());
        self.add_transfer[4..8].copy_from_slice(&b.to_le_bytes());
        self.add_transfer[8..12].copy_from_slice(&loop_count.to_le_bytes());
        self.add_transfer[12..16].copy_from_slice(&0i32.to_le_bytes());

        self.memory
            .write(&mut self.store, self.ptr as usize, &self.add_transfer)
            .expect("raw add(loop=1): memory write に失敗しました");
        let out_len = self
            .call_add
            .call(&mut self.store, (self.ptr, ADD_RAW_TRANSFER_BYTES as i32))
            .expect("raw add(loop=1): 関数呼び出しに失敗しました");
        assert_eq!(
            out_len as usize, ADD_RAW_TRANSFER_BYTES,
            "raw add(loop=1): output length mismatch"
        );
        self.memory
            .read(&self.store, self.ptr as usize, &mut self.add_transfer)
            .expect("raw add(loop=1): memory read に失敗しました");
        let result = i32::from_le_bytes(
            self.add_transfer[12..16]
                .try_into()
                .expect("raw add(loop=1): result decode に失敗しました"),
        );
        black_box(result);
    }

    fn bench_add_raw_loop2000_once(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        let a = (self.tick % 1024) as i32;
        let b = ((self.tick * 3) % 1024) as i32;
        let loop_count = 2000i32;
        self.add_transfer[0..4].copy_from_slice(&a.to_le_bytes());
        self.add_transfer[4..8].copy_from_slice(&b.to_le_bytes());
        self.add_transfer[8..12].copy_from_slice(&loop_count.to_le_bytes());
        self.add_transfer[12..16].copy_from_slice(&0i32.to_le_bytes());

        self.memory
            .write(&mut self.store, self.ptr as usize, &self.add_transfer)
            .expect("raw add(loop=2000): memory write に失敗しました");
        let out_len = self
            .call_add
            .call(&mut self.store, (self.ptr, ADD_RAW_TRANSFER_BYTES as i32))
            .expect("raw add(loop=2000): 関数呼び出しに失敗しました");
        assert_eq!(
            out_len as usize, ADD_RAW_TRANSFER_BYTES,
            "raw add(loop=2000): output length mismatch"
        );
        self.memory
            .read(&self.store, self.ptr as usize, &mut self.add_transfer)
            .expect("raw add(loop=2000): memory read に失敗しました");
        let result = i32::from_le_bytes(
            self.add_transfer[12..16]
                .try_into()
                .expect("raw add(loop=2000): result decode に失敗しました"),
        );
        black_box(result);
    }
}

fn build_payload(size: usize) -> Vec<u8> {
    (0..size)
        .map(|index| ((index * 31 + 17) % 251) as u8)
        .collect()
}

#[inline(never)]
fn add_native(a: i32, b: i32, loop_count: i32) -> i32 {
    if loop_count <= 0 {
        return 0;
    }

    let mut sum = 0;
    let mut sink = 0;
    for _ in 0..loop_count {
        sum += a + b;
        // 反復ごとの処理を最適化で潰さないため、volatile write を行う。
        unsafe {
            std::ptr::write_volatile(&mut sink, sum);
        }
    }

    unsafe {
        std::ptr::read_volatile(&sink);
    }

    sum
}

fn mutate_payload(payload: &mut [u8], tick: u64) {
    if payload.is_empty() {
        return;
    }
    payload[0] = (tick % 251) as u8;
    payload[payload.len() - 1] = ((tick * 3) % 251) as u8;
}

fn build_update_input() -> Vec<SensorData> {
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

fn bench_wit_component(c: &mut Criterion) {
    let component_path = plugin_path("control.component.wasm");
    assert!(
        component_path.exists(),
        "component Wasm が見つかりません: {}。先に `make -C examples/moonbit-runner build-plugin` を実行してください",
        component_path.display()
    );

    let mut harness = WitHarness::new(&component_path);
    let mut group = c.benchmark_group("moonbit-runner/wit");

    group.bench_function("update", |b| b.iter(|| harness.bench_update_once()));
    group.bench_function("add_loop1", |b| b.iter(|| harness.bench_add_loop1_once()));
    group.bench_function("add_loop2000", |b| {
        b.iter(|| harness.bench_add_heavy_once())
    });

    group.throughput(Throughput::Bytes(128));
    group.bench_with_input(BenchmarkId::new("benchmark", "128B"), &128usize, |b, _| {
        b.iter(|| harness.bench_128_once())
    });

    group.throughput(Throughput::Bytes(1024));
    group.bench_with_input(BenchmarkId::new("benchmark", "1KB"), &1024usize, |b, _| {
        b.iter(|| harness.bench_1k_once())
    });

    group.throughput(Throughput::Bytes(4096));
    group.bench_with_input(BenchmarkId::new("benchmark", "4KB"), &4096usize, |b, _| {
        b.iter(|| harness.bench_4k_once())
    });

    group.finish();
}

fn bench_raw_linear_memory(c: &mut Criterion) {
    if std::env::var("MOONBIT_RUNNER_RAW_BENCH").ok().as_deref() != Some("1") {
        eprintln!("MOONBIT_RUNNER_RAW_BENCH=1 が未指定のため raw benchmark をスキップします");
        return;
    }

    let raw_path = plugin_path("control.core.wasm");
    if !raw_path.exists() {
        eprintln!(
            "raw Wasm が存在しないため raw benchmark をスキップします: {}",
            raw_path.display()
        );
        return;
    }

    let mut harness = RawHarness::new(&raw_path);
    let mut group = c.benchmark_group("moonbit-runner/raw");

    group.throughput(Throughput::Bytes(128));
    group.bench_with_input(
        BenchmarkId::new("benchmark_raw", "128B"),
        &128usize,
        |b, _| b.iter(|| harness.bench_128_once()),
    );

    group.throughput(Throughput::Bytes(1024));
    group.bench_with_input(
        BenchmarkId::new("benchmark_raw", "1KB"),
        &1024usize,
        |b, _| b.iter(|| harness.bench_1k_once()),
    );

    group.throughput(Throughput::Bytes(4096));
    group.bench_with_input(
        BenchmarkId::new("benchmark_raw", "4KB"),
        &4096usize,
        |b, _| b.iter(|| harness.bench_4k_once()),
    );

    group.throughput(Throughput::Bytes(ADD_RAW_TRANSFER_BYTES as u64));
    group.bench_with_input(
        BenchmarkId::new("add_raw", "loop1-16B"),
        &ADD_RAW_TRANSFER_BYTES,
        |b, _| b.iter(|| harness.bench_add_raw_loop1_once()),
    );

    group.throughput(Throughput::Bytes(ADD_RAW_TRANSFER_BYTES as u64));
    group.bench_with_input(
        BenchmarkId::new("add_raw", "loop2000-16B"),
        &ADD_RAW_TRANSFER_BYTES,
        |b, _| b.iter(|| harness.bench_add_raw_loop2000_once()),
    );

    group.finish();
}

fn bench_native_add(c: &mut Criterion) {
    let mut harness = NativeHarness::new();
    let mut group = c.benchmark_group("moonbit-runner/native");

    group.bench_function("add_loop1", |b| b.iter(|| harness.bench_add_loop1_once()));
    group.bench_function("add_loop2000", |b| {
        b.iter(|| harness.bench_add_loop2000_once())
    });

    group.finish();
}

criterion_group!(
    moonbit_runner_benches,
    bench_wit_component,
    bench_raw_linear_memory,
    bench_native_add
);
criterion_main!(moonbit_runner_benches);
