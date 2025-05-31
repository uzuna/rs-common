pub mod hello {
    // crate rootからの相対パスで指定
    wasmtime::component::bindgen!(in "../../wasm-comp/hello/wit/world.wit");

    pub use exports::local::hello::types::Pos2;
    use wasmtime::{component::ResourceAny, Store};

    // インスタンスを作るタイプは、関数呼び出しと違ってインスタンスを使い回さなければならない
    pub struct SetterWrap {
        instance: Example,
        setter: ResourceAny,
    }

    impl SetterWrap {
        pub fn new(instance: Example, setter: ResourceAny) -> Self {
            Self { instance, setter }
        }

        pub fn get<T>(&self, store: &mut Store<T>) -> anyhow::Result<Pos2> {
            let g = self.instance.local_hello_types();
            let caller = g.setter();
            let res = caller.call_get(store, self.setter)?;
            Ok(res)
        }

        pub fn set<T>(&self, store: &mut Store<T>, p: Pos2) -> anyhow::Result<()> {
            let g = self.instance.local_hello_types();
            let caller = g.setter();
            caller.call_set(store, self.setter, p)?;
            Ok(())
        }

        // 自動でドロップされない
        pub fn drop<T>(self, store: &mut Store<T>) -> anyhow::Result<()> {
            self.setter.resource_drop(store)?;
            Ok(())
        }
    }
}

pub mod hasdep {
    wasmtime::component::bindgen!(in "wit-front/world.wit");
}

pub mod calc {
    wasmtime::component::bindgen!(in "../../wasm-comp/calc/wit/world.wit");
}
