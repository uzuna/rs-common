//! ロボット制御ホストの公開API。
//! 統合テストから `plugin_manager` モジュールを利用できるようにするために公開する。

pub mod plugin_manager;
pub mod so_plugin_handle;
pub mod version_manager;

pub use plugin_manager::PluginRouter;
pub use so_plugin_handle::SoPluginHandle;
