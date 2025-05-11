pub mod camera;
pub mod colored;
pub(crate) mod common;
pub mod gltf;
pub mod graph;
pub mod model;
pub mod particle;
pub mod prelude;
pub mod texture;
pub mod tutorial;
pub mod types;
pub mod uniform;
pub mod util;
pub mod vertex;

pub trait WgpuContext {
    // deviceを返す。リソースの作成に必要
    fn device(&self) -> &wgpu::Device;
    // レンダリングパス作成に必要
    fn surface(&self) -> &wgpu::Surface;
    // コマンドの実行に必要。レンダリングパスと同時に使ったり、Uniformの設定に使ったり
    fn queue(&self) -> &wgpu::Queue;
    // 1画面だけのレンダリングならコンテキストが1枚持っていれば十分
    fn depth(&self) -> &wgpu::TextureView;
}
