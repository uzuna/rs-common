pub mod introduction;
pub mod particle;
pub mod prelude;

pub trait WgpuContext {
    fn device(&self) -> &wgpu::Device;
    fn surface(&self) -> &wgpu::Surface;
    fn queue(&self) -> &wgpu::Queue;
}
