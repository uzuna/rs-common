use criterion::{criterion_group, criterion_main, Criterion};
use wasmtime::{component::Component, Engine, Store};

wasmtime::component::bindgen!(in "wit/world.wit");

struct WasmComponent<T> {
    store: Store<T>,
    component: Component,
    linker: wasmtime::component::Linker<T>,
}

impl<T> WasmComponent<T> {
    // WASIなしで実行する場合のコンポーネントを生成
    fn new_unknown(engine: &Engine, byte: &[u8], data: T) -> anyhow::Result<Self> {
        // WASIなしで実行する場合の設定
        let store = Store::new(engine, data);
        let component = Component::new(engine, byte)?;
        let linker = wasmtime::component::Linker::new(engine);
        Ok(Self {
            store,
            component,
            linker,
        })
    }

    fn instance(&mut self) -> anyhow::Result<Example> {
        // コンポーネントをインスタンス化
        let res = Example::instantiate(&mut self.store, &self.component, &self.linker)?;
        Ok(res)
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

fn benchmark_calculate_add(c: &mut Criterion) {
    let wasm_binaly =
        std::fs::read("../../target/wasm32-unknown-unknown/release/wasm_plugin_hello.wasm")
            .expect("Failed to read wasm file");
    // wasmtimeのエンジンを初期化
    let engine = Engine::default();
    let mut ctx = WasmComponent::new_unknown(&engine, &wasm_binaly, ())
        .expect("Failed to create WasmComponent");
    let instance = ctx.instance().expect("Failed to instantiate component");

    c.bench_function("Rust add", |b| b.iter(|| rust_calculate_add(1, 2)));

    c.bench_function("Wasm add", |b| {
        b.iter(|| {
            instance
                .call_add(&mut ctx.store, 1, 2)
                .expect("Failed to call add function")
        })
    });

    let len: u32 = 1000;
    let list = (0..len).collect::<Vec<u32>>();

    c.bench_function(&format!("Rust list_sum({len})"), |b| {
        b.iter(|| rust_list_sum(&list))
    });
    c.bench_function(&format!("Wasm list_sum({len})"), |b| {
        b.iter(|| {
            instance
                .call_sum(&mut ctx.store, &list)
                .expect("Failed to call sum function")
        })
    });

    c.bench_function(&format!("Rust loop_sum({len})"), |b| {
        b.iter(|| rust_loop_sum(len))
    });
    c.bench_function(&format!("Wasm loop_sum({len})"), |b| {
        b.iter(|| {
            instance
                .call_loop_sum(&mut ctx.store, len)
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
            instance
                .call_generate_string(&mut ctx.store, len)
                .expect("Failed to call generate_string function")
        })
    });
}

criterion_group!(benches, benchmark_calculate_add);
criterion_main!(benches);
