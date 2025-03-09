use wgpu::util::DeviceExt;

pub struct UniformBuffer<V> {
    pub buf: wgpu::Buffer,
    _phantom: std::marker::PhantomData<V>,
}

impl<V> UniformBuffer<V>
where
    V: Default + bytemuck::Pod,
{
    pub fn new(device: &wgpu::Device, label: Option<&str>) -> Self {
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label,
            contents: bytemuck::cast_slice(&[V::default()]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        Self {
            buf: uniform,
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn update(&self, queue: &wgpu::Queue, uni: V) {
        queue.write_buffer(&self.buf, 0, bytemuck::cast_slice(&[uni]));
    }
}

impl<V> UniformBuffer<V> {
    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buf
    }
}
