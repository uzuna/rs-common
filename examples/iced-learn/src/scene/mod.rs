/// システム情報に関する取得、保持、表示を行う
pub mod systeminfo {
    use iced::{
        system,
        widget::{center, column, text},
        Element, Task,
    };

    #[derive(Debug, Clone)]
    pub enum SystemInfoMsg {
        LoadSystemInfo,
        SystemInfoLoaded(Box<system::Information>),
    }

    impl SystemInfoMsg {
        fn loaded(info: system::Information) -> Self {
            SystemInfoMsg::SystemInfoLoaded(Box::new(info))
        }
    }

    /// 情報の保持、メッセージに対応した処理を行う
    pub struct Scene {
        system_info: Option<system::Information>,
    }

    impl Default for Scene {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Scene {
        pub fn new() -> Self {
            Self { system_info: None }
        }

        /// メッセージ処理
        pub fn update(&mut self, message: SystemInfoMsg) -> Task<SystemInfoMsg> {
            match message {
                SystemInfoMsg::LoadSystemInfo => {
                    return system::fetch_information().map(SystemInfoMsg::loaded)
                }
                SystemInfoMsg::SystemInfoLoaded(info) => {
                    self.system_info = Some(*info);
                }
            }
            Task::none()
        }

        /// 表示エレメントを返す
        ///
        /// 大きさとかその他の規定が有るとなお良さそう
        pub fn view<M>(&self) -> Element<M>
        where
            M: Clone + std::fmt::Debug + 'static,
        {
            let content = match &self.system_info {
                Some(info) => Self::element_system_info::<M>(info),
                None => text!("Loading...").into(),
            };

            center(content).into()
        }

        /// システム情報を表示するためのエレメントを生成
        fn element_system_info<M>(info: &system::Information) -> Element<M>
        where
            M: Clone + std::fmt::Debug + 'static,
        {
            use bytesize::ByteSize;

            let content: Element<_> = {
                let system_name = text!(
                    "System name: {}",
                    info.system_name.as_ref().unwrap_or(&"unknown".to_string())
                );

                let system_kernel = text!(
                    "System kernel: {}",
                    info.system_kernel
                        .as_ref()
                        .unwrap_or(&"unknown".to_string())
                );

                let system_version = text!(
                    "System version: {}",
                    info.system_version
                        .as_ref()
                        .unwrap_or(&"unknown".to_string())
                );

                let system_short_version = text!(
                    "System short version: {}",
                    info.system_short_version
                        .as_ref()
                        .unwrap_or(&"unknown".to_string())
                );

                let cpu_brand = text!("Processor brand: {}", info.cpu_brand);

                let cpu_cores = text!(
                    "Processor cores: {}",
                    info.cpu_cores
                        .map_or("unknown".to_string(), |cores| cores.to_string())
                );

                let memory_readable = ByteSize::b(info.memory_total).to_string();

                let memory_total = text!(
                    "Memory (total): {} bytes ({memory_readable})",
                    info.memory_total,
                );

                let memory_text = if let Some(memory_used) = info.memory_used {
                    let memory_readable = ByteSize::b(memory_used).to_string();

                    format!("{memory_used} bytes ({memory_readable})")
                } else {
                    String::from("None")
                };

                let memory_used = text!("Memory (used): {memory_text}");

                let graphics_adapter = text!("Graphics adapter: {}", info.graphics_adapter);

                let graphics_backend = text!("Graphics backend: {}", info.graphics_backend);

                column![
                    system_name,
                    system_kernel,
                    system_version,
                    system_short_version,
                    cpu_brand,
                    cpu_cores,
                    memory_total,
                    memory_used,
                    graphics_adapter,
                    graphics_backend,
                ]
                .spacing(1)
                .into()
            };

            center(content).into()
        }
    }
}
