pub mod hello {
    // crate rootからの相対パスで指定
    wasmtime::component::bindgen!(in "../../wasm-comp/hello/wit/world.wit");

    pub use exports::local::hello::types::Pos2;
    use wasmtime::{
        component::{Component, ResourceAny},
        Store,
    };

    use crate::context::ExecStore;

    /// Hello worldのインスタンス構造体
    pub struct HelloInst<T> {
        /// Componentに定義された関数とメソットでアクセスするための型
        instance: Example,
        /// インsタンスの状態やhostリソースへのアクセスなどすべてのデータを保持するストア
        store: Store<T>,
        // helloで定義したsetterの型表現
        // 今回はインスタンスに結びつけて利用している
        setter: ResourceAny,
    }

    impl<T> HelloInst<T> {
        /// 実行環境ストアとwasmバイナリのバイト列からHelloInstを生成する
        pub fn new_with_binary(es: ExecStore<T>, component: &Component) -> anyhow::Result<Self> {
            let ExecStore { mut store, linker } = es;
            let e = Example::instantiate(&mut store, component, &linker)?;
            let g = e.local_hello_types();
            let setter = g.setter();
            let setter = setter.call_new(&mut store)?;
            Ok(Self::new(e, setter, store))
        }

        /// HelloInstを生成に必要な情報
        pub fn new(instance: Example, setter: ResourceAny, store: Store<T>) -> Self {
            Self {
                instance,
                setter,
                store,
            }
        }

        /// hello_world公開関数を呼び出す
        pub fn hello_world(&mut self) -> anyhow::Result<String> {
            let res = self.instance.call_hello_world(&mut self.store)?;
            Ok(res)
        }

        /// add関数を呼び出す
        pub fn add(&mut self, a: u32, b: u32) -> anyhow::Result<u32> {
            let res = self.instance.call_add(&mut self.store, a, b)?;
            Ok(res)
        }

        /// sum関数を呼び出す
        pub fn sum(&mut self, v: &[u32]) -> anyhow::Result<u32> {
            let res = self.instance.call_sum(&mut self.store, v)?;
            Ok(res)
        }

        /// setterのget関数を呼び出す
        pub fn get(&mut self) -> anyhow::Result<Pos2> {
            let g = self.instance.local_hello_types();
            let caller = g.setter();
            let res = caller.call_get(&mut self.store, self.setter)?;
            Ok(res)
        }

        /// setterのset関数を呼び出す
        pub fn set(&mut self, p: Pos2) -> anyhow::Result<()> {
            let g = self.instance.local_hello_types();
            let caller = g.setter();
            caller.call_set(&mut self.store, self.setter, p)?;
            Ok(())
        }
    }

    impl<T> Drop for HelloInst<T> {
        fn drop(&mut self) {
            // リソース型は明示的に破棄するインスタンスのリソースを解放する
            match self.setter.resource_drop(&mut self.store) {
                Ok(_) => {}
                Err(e) => {
                    eprintln!("Failed to drop setter resource: {}", e);
                }
            }
        }
    }

    /// インスタンスの動作確認
    pub fn demo<T>(inst: &mut HelloInst<T>) -> anyhow::Result<()> {
        let res = inst.hello_world()?;
        println!("Hello from WASI Preview1: {}", res);

        for i in 0..5 {
            let result = inst.add(i, i)?;
            println!("add({i}+{i}) = {result}");
        }

        let s = inst.sum(&[1, 2, 3, 4, 5])?;
        println!("sum([1, 2, 3, 4, 5]) = {}", s);

        let res = inst.get()?;
        println!("setter.get() = {:?}", res);
        inst.set(Pos2 { x: 1.0, y: 2.0 })?;
        let get = inst.get()?;
        println!("setter.get() = {:?}", get);

        Ok(())
    }
}

pub mod hasdep {
    use wasmtime::component::Component;

    use crate::context::ExecStore;

    wasmtime::component::bindgen!(in "wit-front/world.wit");

    pub struct HasdepInst<T> {
        /// Componentに定義された関数とメソットでアクセスするための型
        instance: Hasdep,
        /// インスタンスの状態やhostリソースへのアクセスなどすべてのデータを保持するストア
        store: wasmtime::Store<T>,
    }

    impl<T> HasdepInst<T> {
        /// 実行環境ストアとwasmバイナリのバイト列からHasdepInstを生成する
        pub fn new_with_binary(es: ExecStore<T>, component: &Component) -> anyhow::Result<Self> {
            let ExecStore { mut store, linker } = es;
            let e = Hasdep::instantiate(&mut store, component, &linker)?;
            Ok(Self::new(e, store))
        }

        /// HasdepInstを生成に必要な情報
        pub fn new(instance: Hasdep, store: wasmtime::Store<T>) -> Self {
            Self { instance, store }
        }

        /// add関数を呼び出す
        pub fn add(&mut self, a: u32, b: u32) -> anyhow::Result<u32> {
            let res = self.instance.call_add(&mut self.store, a, b)?;
            Ok(res)
        }
    }

    /// インスタンスの動作確認
    pub fn demo<T>(inst: &mut HasdepInst<T>) -> anyhow::Result<()> {
        for i in 0..5 {
            let result = inst.add(i, i)?;
            println!("add({i}+{i}) = {result}");
        }
        Ok(())
    }
}

pub mod calc {
    wasmtime::component::bindgen!(in "../../wasm-comp/calc/wit/world.wit");
}
