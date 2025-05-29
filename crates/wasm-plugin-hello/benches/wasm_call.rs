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

    let summer = instance.component_wasm_plugin_hello_types().summer();
    let r = summer
        .call_new(&mut ctx.store)
        .expect("Failed to call new function on summer resource");
    summer
        .call_set_val(&mut ctx.store, r, &list)
        .expect("Failed to call set function on summer");
    c.bench_function(&format!("Wasm list_sum({len}) in Resource"), |b| {
        b.iter(|| {
            summer
                .call_sum(&mut ctx.store, r)
                .expect("Failed to call sum function on summer")
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

    summer
        .call_set_key(&mut ctx.store, r, "UTY3z2qNfWuWvV5VoFxwpZvymfAxZwyt")
        .expect("Failed to call set_key function on summer");
    c.bench_function(&format!("Wasm generate_string({len}) return only"), |b| {
        b.iter(|| {
            summer
                .call_get_key(&mut ctx.store, r)
                .expect("Failed to call get_key function on summer")
        })
    });
}

fn benchmark_filter(c: &mut Criterion) {
    let wasm_binaly =
        std::fs::read("../../target/wasm32-unknown-unknown/release/wasm_plugin_hello.wasm")
            .expect("Failed to read wasm file");
    // wasmtimeのエンジンを初期化
    let engine = Engine::default();
    let mut ctx = WasmComponent::new_unknown(&engine, &wasm_binaly, ())
        .expect("Failed to create WasmComponent");
    let instance = ctx.instance().expect("Failed to instantiate component");

    let filter = instance.component_wasm_plugin_hello_filter().fir();
    let resource = filter
        .call_new_moving(&mut ctx.store, 8)
        .expect("Failed to call new_moving function on fir resource");

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
                filter
                    .call_filter(&mut ctx.store, resource, x)
                    .expect("Failed to call filter function on fir");
            }
        })
    });

    // まとめて計算することで早くなるか?
    c.bench_function("Rust filter_vec", |b| b.iter(|| r_fir.filter_vec(&list)));

    c.bench_function("Wasm filter_vec", |b| {
        b.iter(|| {
            filter
                .call_filter_vec(&mut ctx.store, resource, &list)
                .expect("Failed to call filter_vec function on fir")
        })
    });
}

criterion_group!(benches, benchmark_calculate_add, benchmark_filter);
criterion_main!(benches);
