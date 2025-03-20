pub mod introduction;
pub mod model;
pub mod particle;
pub mod prelude;
pub mod texture;
pub mod tutorial;
pub mod uniform;
pub mod vertex;

pub trait WgpuContext {
    fn device(&self) -> &wgpu::Device;
    fn surface(&self) -> &wgpu::Surface;
    fn queue(&self) -> &wgpu::Queue;
}
