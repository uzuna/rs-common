use wasmtime::{component::Component, Engine, Store};
use wasmtime_wasi::{
    p2::{IoView, WasiCtx, WasiCtxBuilder, WasiView},
    ResourceTable,
};

use crate::bingings::{
    hasdep,
    hello::{Example, Pos2, SetterWrap},
};

/// WASIリンクに必要なトレイと実装構造体
///
/// wasip2でコンパイルした場合、実行環境は[WASIp2 interface](https://docs.wasmtime.dev/api/wasmtime_wasi/p2/index.html)を実装している必要がある
/// この実装を保持して提供する役目がある。
pub struct Preview2Host {
    wasi_ctx: WasiCtx,
    resource_table: ResourceTable,
}

impl Default for Preview2Host {
    fn default() -> Self {
        let wasi_ctx = WasiCtxBuilder::new().inherit_stdio().build();
        let resource_table = ResourceTable::default();
        Self {
            wasi_ctx,
            resource_table,
        }
    }
}

/// [WasiView]は[IoView]トレイトを前提としている
impl IoView for Preview2Host {
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.resource_table
    }
}

/// [wasmtime_wasi::p2::add_to_linker_sync]を実行するためには[WasiView]トレイトの実装が必要
impl WasiView for Preview2Host {
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi_ctx
    }
}

pub struct WasmComponent<T> {
    store: Store<T>,
    component: Component,
    linker: wasmtime::component::Linker<T>,
}

impl<T> WasmComponent<T> {
    // WASIなしで実行する場合のコンポーネントを生成
    pub fn new_unknown(engine: &Engine, byte: &[u8], data: T) -> anyhow::Result<Self> {
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

    pub fn hello_instance(&mut self) -> anyhow::Result<Example> {
        // コンポーネントをインスタンス化
        let res = Example::instantiate(&mut self.store, &self.component, &self.linker)?;
        Ok(res)
    }

    pub fn setter(&mut self) -> anyhow::Result<SetterWrap> {
        let e = Example::instantiate(&mut self.store, &self.component, &self.linker)?;
        let g = e.local_hello_types();
        let caller = g.setter();
        let setter = caller.call_new(&mut self.store)?;
        let sw = SetterWrap::new(e, setter);
        Ok(sw)
    }

    pub fn hasdep_instance(&mut self) -> anyhow::Result<hasdep::Hasdep> {
        // hasdepコンポーネントをインスタンス化
        let res = hasdep::Hasdep::instantiate(&mut self.store, &self.component, &self.linker)?;
        Ok(res)
    }
}

impl WasmComponent<Preview2Host> {
    pub fn new_p2(engine: &Engine, byte: &[u8]) -> anyhow::Result<Self> {
        // WASI Preview2のインターフェースを追加
        let store = Store::new(engine, Preview2Host::default());
        let component = Component::new(engine, byte)?;
        let mut linker = wasmtime::component::Linker::new(engine);
        // 重複読み出しを許可
        linker.allow_shadowing(true);
        // WASIp2向けのインターフェースをリンカーに追加
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;

        Ok(Self {
            store,
            component,
            linker,
        })
    }
}

pub fn run_sequence_hello<T>(c: &mut WasmComponent<T>) -> anyhow::Result<()> {
    let e = c.hello_instance()?;
    let res = e.call_hello_world(&mut c.store)?;
    println!("Hello from WASI Preview1: {}", res);

    for i in 0..5 {
        let result = e.call_add(&mut c.store, i, i)?;
        println!("add({i}+{i}) = {result}");
    }

    let s = e.call_sum(&mut c.store, &[1, 2, 3, 4, 5])?;
    println!("sum([1, 2, 3, 4, 5]) = {}", s);

    let sw = c.setter()?;
    let res = sw.get(&mut c.store)?;
    println!("setter.get() = {:?}", res);
    sw.set(&mut c.store, Pos2 { x: 1.0, y: 2.0 })?;
    let get = sw.get(&mut c.store)?;
    println!("setter.get() = {:?}", get);
    sw.drop(&mut c.store)?;

    Ok(())
}

pub fn run_hasdep<T>(c: &mut WasmComponent<T>) -> anyhow::Result<()> {
    let e = c.hasdep_instance()?;

    for i in 0..5 {
        let result = e.call_add(&mut c.store, i, i)?;
        println!("add({i}+{i}) = {result}");
    }

    Ok(())
}
