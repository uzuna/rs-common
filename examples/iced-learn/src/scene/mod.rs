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

pub mod title_text {
    use iced::{widget::text_input, Element};

    use super::ltm::Store;

    #[derive(Debug, Clone)]
    pub enum TitleTextMsg {
        Load,
        Changes(String),
        Submit(String),
    }

    pub struct Scene {
        pub text: String,
    }

    impl Default for Scene {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Scene {
        pub fn new() -> Self {
            Self {
                text: "Iced Learn".to_string(),
            }
        }

        pub fn title(&self) -> &str {
            &self.text
        }

        pub fn update(&mut self, ltm: &Store, message: TitleTextMsg) {
            match message {
                TitleTextMsg::Load => {
                    if let Ok(Some(text)) = ltm.get::<String>("title") {
                        self.text = text;
                    } else {
                        self.text = "Iced Learn".to_string();
                    }
                }
                TitleTextMsg::Changes(text) => {
                    self.text = text;
                }
                // 以前の記録を保持
                TitleTextMsg::Submit(text) => {
                    self.text = text.clone();
                    if let Err(e) = ltm.insert("title", &text) {
                        tracing::error!("Failed to insert title: {}", e);
                    }
                }
            }
        }

        pub fn view<M>(&self) -> Element<M>
        where
            M: Clone + std::fmt::Debug + 'static + From<TitleTextMsg>,
        {
            use iced::widget::column;

            let content = column![text_input("Type something here...", &self.text)
                .on_input(|text| { M::from(TitleTextMsg::Changes(text)) })
                .on_submit(M::from(TitleTextMsg::Submit(self.text.clone())))
                .size(14)
                .padding(20)];

            content.into()
        }
    }
}

/// 操作記録や設定保存を行うためのモジュール
pub mod ltm {

    pub struct Store {
        conn: rusqlite::Connection,
    }

    // 単純なKey-ValueストアのAPIで実装
    impl Store {
        pub fn new() -> anyhow::Result<Self> {
            let base_dir = dirs::config_dir()
                .ok_or_else(|| anyhow::anyhow!("Failed to get config directory"))?
                .join("iced-learn");

            std::fs::create_dir_all(&base_dir)
                .map_err(|e| anyhow::anyhow!("Failed to create config directory: {}", e))?;
            let path = base_dir.join("store.db");
            let conn = rusqlite::Connection::open(path)?;
            Ok(Self { conn })
        }

        pub fn create_table(&self) -> anyhow::Result<()> {
            self.conn.execute(
                "CREATE TABLE IF NOT EXISTS kvs (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                )",
                [],
            )?;
            Ok(())
        }

        pub fn insert<T>(&self, key: &str, value: &T) -> anyhow::Result<()>
        where
            T: serde::Serialize,
        {
            let value = serde_json::to_string(value)
                .map_err(|e| anyhow::anyhow!("Failed to serialize value: {}", e))?;
            self.conn.execute(
                "INSERT INTO kvs (key, value) VALUES (?1, ?2)",
                rusqlite::params![key, value],
            )?;
            Ok(())
        }

        pub fn get<T>(&self, key: &str) -> anyhow::Result<Option<T>>
        where
            T: serde::de::DeserializeOwned,
        {
            let mut stmt = self.conn.prepare("SELECT value FROM kvs WHERE key = ?1")?;
            let mut rows = stmt.query(rusqlite::params![key])?;
            if let Some(row) = rows.next()? {
                let value: String = row.get(0)?;
                let value = serde_json::from_str(&value)
                    .map_err(|e| anyhow::anyhow!("Failed to deserialize value: {}", e))?;
                Ok(Some(value))
            } else {
                Ok(None)
            }
        }

        pub fn delete(&self, key: &str) -> anyhow::Result<()> {
            self.conn
                .execute("DELETE FROM kvs WHERE key = ?1", rusqlite::params![key])?;
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_store() -> anyhow::Result<()> {
            let store = Store::new()?;
            store.create_table()?;

            let key = "test_key";
            let value = "test_value";

            store.insert(key, &value)?;

            let retrieved_value: Option<String> = store.get(key)?;
            assert_eq!(retrieved_value, Some(value.to_string()));

            store.delete(key)?;

            let retrieved_value: Option<String> = store.get(key)?;
            assert_eq!(retrieved_value, None);

            Ok(())
        }
    }
}
