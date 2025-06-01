use std::path::Path;

use anyhow::Context;
use criterion::{criterion_group, criterion_main, Criterion};
use wasmtime::{
    component::{Component, ResourceAny},
    Engine, Store,
};

wasmtime::component::bindgen!(in "wit/world.wit");

const EXAMPLE_WASM: &str = "../../target/wasm32-unknown-unknown/release/hello.wasm";

// refer: examples/wasmtime-runner/src/bindings.rs
struct ExecStore<T> {
    store: Store<T>,
    linker: wasmtime::component::Linker<T>,
}

impl<T> ExecStore<T> {
    // WASIなしで実行する場合のコンポーネントを生成
    fn new_core(engine: &Engine, data: T) -> Self {
        let store = Store::new(engine, data);
        let linker = wasmtime::component::Linker::new(engine);
        Self { store, linker }
    }
}

fn load_wasm_component(engine: &Engine) -> anyhow::Result<Component> {
    load_from_file(engine, EXAMPLE_WASM)
}

fn load_from_file(engine: &Engine, file: impl AsRef<Path>) -> anyhow::Result<Component> {
    let buffer = std::fs::read(&file)
        .with_context(|| format!("Failed to read wasm file: {}", file.as_ref().display()))?;
    Component::new(engine, &buffer).with_context(|| {
        format!(
            "Failed to create component from file: {}",
            file.as_ref().display()
        )
    })
}

struct HelloInst<T> {
    instance: Example,
    store: Store<T>,
    summer: ResourceAny,
}

impl<T> HelloInst<T> {
    fn new_with_binary(es: ExecStore<T>, component: &Component) -> anyhow::Result<Self> {
        let ExecStore { mut store, linker } = es;
        let e = Example::instantiate(&mut store, component, &linker)?;
        let g = e.local_hello_types();
        let summer = g.summer();
        let summer = summer.call_new(&mut store)?;
        Ok(Self::new(e, store, summer))
    }

    fn new(instance: Example, store: Store<T>, summer: ResourceAny) -> Self {
        Self {
            instance,
            store,
            summer,
        }
    }

    fn call_add(&mut self, a: u32, b: u32) -> anyhow::Result<u32> {
        self.instance.call_add(&mut self.store, a, b)
    }

    fn call_sum(&mut self, v: &[u32]) -> anyhow::Result<u32> {
        self.instance.call_sum(&mut self.store, v)
    }

    fn call_loop_sum(&mut self, len: u32) -> anyhow::Result<u32> {
        self.instance.call_loop_sum(&mut self.store, len)
    }

    fn call_generate_string(&mut self, len: u32) -> anyhow::Result<String> {
        self.instance.call_generate_string(&mut self.store, len)
    }

    fn summer_set_key(&mut self, key: &str) -> anyhow::Result<()> {
        let g = self.instance.local_hello_types();
        let summer = g.summer();
        summer.call_set_key(&mut self.store, self.summer, key)?;
        Ok(())
    }

    fn summer_get_key(&mut self) -> anyhow::Result<String> {
        let g = self.instance.local_hello_types();
        let summer = g.summer();
        let key = summer.call_get_key(&mut self.store, self.summer)?;
        Ok(key)
    }

    fn summer_set_val(&mut self, val: &[u32]) -> anyhow::Result<()> {
        let g = self.instance.local_hello_types();
        let summer = g.summer();
        summer.call_set_val(&mut self.store, self.summer, val)?;
        Ok(())
    }

    fn summer_sum(&mut self) -> anyhow::Result<u32> {
        let g = self.instance.local_hello_types();
        let summer = g.summer();
        let sum = summer.call_sum(&mut self.store, self.summer)?;
        Ok(sum)
    }
}

struct HelloInstFir<T> {
    // Wit定義に従ってComponentを呼ぶための型
    instance: Example,
    // wasmインスタンスデータ保持構造体
    store: Store<T>,
    // WASMインスタンス内のFIRリソースクラス型定義情報
    fir: ResourceAny,
}

impl<T> HelloInstFir<T> {
    fn new_with_binary(es: ExecStore<T>, component: &Component) -> anyhow::Result<Self> {
        let ExecStore { mut store, linker } = es;
        let e = Example::instantiate(&mut store, component, &linker)?;
        let g = e.local_hello_filter();
        let fir = g.fir();
        let fir = fir.call_new_moving(&mut store, 8)?;
        Ok(Self::new(e, store, fir))
    }

    fn new(instance: Example, store: Store<T>, fir: ResourceAny) -> Self {
        Self {
            instance,
            store,
            fir,
        }
    }

    fn call_filter(&mut self, x: f32) -> anyhow::Result<f32> {
        let g = self.instance.local_hello_filter();
        let fir = g.fir();
        fir.call_filter(&mut self.store, self.fir, x)
    }

    fn call_filter_vec(&mut self, list: &[f32]) -> anyhow::Result<Vec<f32>> {
        let g = self.instance.local_hello_filter();
        let fir = g.fir();
        fir.call_filter_vec(&mut self.store, self.fir, list)
    }
}

// 1CPU命令レベルの大きさ
fn rust_calculate_add(a: u32, b: u32) -> u32 {
    a + b
}

// メモリ転送含めた処理の長さ
fn rust_list_sum(l: &[u32]) -> u32 {
    l.iter().sum()
}

// 内部最適化を含めた純粋な計算の長さ。listと比べることでメモリアクセスオーバーヘッドの差が見えるはず
fn rust_loop_sum(len: u32) -> u32 {
    (0..len).sum()
}

fn rust_generate_string(len: usize) -> String {
    let mut s = String::with_capacity(len);
    for i in 0..len {
        s.push_str(&i.to_string());
    }
    s
}

fn benchmark_calculate_add_inner(c: &mut Criterion) -> anyhow::Result<()> {
    let engine = Engine::default();
    let comp = load_wasm_component(&engine)?;
    let es = ExecStore::new_core(&engine, ());
    let mut inst = HelloInst::new_with_binary(es, &comp)?;

    c.bench_function("Rust add", |b| b.iter(|| rust_calculate_add(1, 2)));

    c.bench_function("Wasm add", |b| {
        b.iter(|| inst.call_add(1, 2).expect("Failed to call add function"))
    });

    let len: u32 = 1000;
    let list = (0..len).collect::<Vec<u32>>();

    c.bench_function(&format!("Rust list_sum({len})"), |b| {
        b.iter(|| rust_list_sum(&list))
    });
    c.bench_function(&format!("Wasm list_sum({len})"), |b| {
        b.iter(|| inst.call_sum(&list).expect("Failed to call sum function"))
    });

    inst.summer_set_val(&list)?;
    c.bench_function(&format!("Wasm list_sum({len}) in Resource"), |b| {
        b.iter(|| {
            inst.summer_sum()
                .expect("Failed to call sum function on summer")
        })
    });

    c.bench_function(&format!("Rust loop_sum({len})"), |b| {
        b.iter(|| rust_loop_sum(len))
    });
    c.bench_function(&format!("Wasm loop_sum({len})"), |b| {
        b.iter(|| {
            inst.call_loop_sum(len)
                .expect("Failed to call loop_sum function")
        })
    });

    // 何らかの項目名などを生成するイメージ
    let len: u32 = 32;

    c.bench_function(&format!("Rust generate_string({len})"), |b| {
        b.iter(|| rust_generate_string(len as usize))
    });
    c.bench_function(&format!("Wasm generate_string({len})"), |b| {
        b.iter(|| {
            inst.call_generate_string(len)
                .expect("Failed to call generate_string function")
        })
    });

    inst.summer_set_key("UTY3z2qNfWuWvV5VoFxwpZvymfAxZwyt")?;
    c.bench_function(&format!("Wasm generate_string({len}) return only"), |b| {
        b.iter(|| {
            inst.summer_get_key()
                .expect("Failed to call get_key function on summer")
        })
    });
    Ok(())
}

fn benchmark_filter_inner(c: &mut Criterion) -> anyhow::Result<()> {
    let engine = Engine::default();
    let comp = load_wasm_component(&engine)?;
    let es = ExecStore::new_core(&engine, ());
    let mut inst = HelloInstFir::new_with_binary(es, &comp)?;

    let mut r_fir = dsp::Fir::new_moving(8);

    let list = (0..1000)
        .map(|i| (i as f32 * 0.01).sin())
        .collect::<Vec<f32>>();

    c.bench_function("Rust filter", |b| {
        b.iter(|| {
            for &x in &list {
                r_fir.filter(x);
            }
        })
    });

    c.bench_function("Wasm filter", |b| {
        b.iter(|| {
            for &x in &list {
                inst.call_filter(x)
                    .expect("Failed to call filter function on fir");
            }
        })
    });

    // まとめて計算することで早くなるか?
    c.bench_function("Rust filter_vec", |b| b.iter(|| r_fir.filter_vec(&list)));

    c.bench_function("Wasm filter_vec", |b| {
        b.iter(|| {
            inst.call_filter_vec(&list)
                .expect("Failed to call filter_vec function on fir")
        })
    });
    Ok(())
}

fn benchmark_calculate_add(c: &mut Criterion) {
    benchmark_calculate_add_inner(c).unwrap_or_else(|e| eprintln!("Benchmark failed: {}", e));
}

fn benchmark_filter(c: &mut Criterion) {
    benchmark_filter_inner(c).unwrap_or_else(|e| eprintln!("Benchmark failed: {}", e));
}

criterion_group!(benches, benchmark_calculate_add, benchmark_filter);
criterion_main!(benches);
