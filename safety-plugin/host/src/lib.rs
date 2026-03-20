//! ロボット制御ホストの公開API。
//! 統合テストから `plugin_manager` モジュールを利用できるようにするために公開する。

pub mod plugin_manager;
pub mod version_manager;

pub use plugin_manager::PluginRouter;
