pub struct VertexBuffer<V> {
    pub buf: wgpu::Buffer,
    pub index: wgpu::Buffer,
    index_len: usize,
    phantom: std::marker::PhantomData<V>,
}

impl<V> VertexBuffer<V>
where
    V: bytemuck::Pod,
{
    pub fn new(device: &wgpu::Device, verts: &[V], indexes: &[u16]) -> Self {
        use wgpu::util::DeviceExt;
        let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: bytemuck::cast_slice(verts),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let index = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Index Buffer"),
            contents: bytemuck::cast_slice(indexes),
            usage: wgpu::BufferUsages::INDEX,
        });
        let index_len = indexes.len();
        Self {
            buf,
            index,
            index_len,
            phantom: std::marker::PhantomData,
        }
    }

    pub fn update(&self, queue: &wgpu::Queue, verts: &[V]) {
        queue.write_buffer(&self.buf, 0, bytemuck::cast_slice(verts));
    }

    pub(crate) fn draw(&self, rpass: &mut wgpu::RenderPass, instance_range: std::ops::Range<u32>) {
        rpass.set_vertex_buffer(0, self.buf.slice(..));
        rpass.set_index_buffer(self.index.slice(..), wgpu::IndexFormat::Uint16);
        rpass.draw_indexed(0..self.index_len as u32, 0, instance_range);
    }
}
