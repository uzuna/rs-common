use std::ops::Range;

use wgpu::util::DeviceExt;

pub struct VertexBuffer<V> {
    pub vert: wgpu::Buffer,
    // 全長が変わらない想定
    vert_len: usize,
    phantom: std::marker::PhantomData<V>,
}

impl<V> VertexBuffer<V>
where
    V: bytemuck::Pod,
{
    pub fn new(device: &wgpu::Device, vertices: &[V], label: Option<&str>) -> Self {
        let vert = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label,
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        Self {
            vert,
            vert_len: vertices.len(),
            phantom: std::marker::PhantomData,
        }
    }

    pub fn update_vertices(&self, queue: &wgpu::Queue, vertices: &[V]) {
        queue.write_buffer(&self.vert, 0, bytemuck::cast_slice(vertices));
    }

    pub fn draw(&self, rpass: &mut wgpu::RenderPass, vert_range: Range<u32>) {
        rpass.set_vertex_buffer(0, self.vert.slice(..));
        rpass.draw(vert_range, 0..self.vert_len as u32);
    }
}
