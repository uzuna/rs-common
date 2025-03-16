use std::ops::Range;

/// 頂点とインデックスで構成するVertexBuffer
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

/// 頂点とインデックスで構成するVertexBuffer
pub struct ViBuffer<V, I> {
    pub vertex: wgpu::Buffer,
    pub index: wgpu::Buffer,
    pub instance: wgpu::Buffer,
    index_len: usize,
    instance_len: usize,
    _p0: std::marker::PhantomData<V>,
    _p1: std::marker::PhantomData<I>,
}

impl<V, I> ViBuffer<V, I>
where
    V: bytemuck::Pod,
    I: bytemuck::Pod,
{
    pub fn new(device: &wgpu::Device, verts: &[V], indexes: &[u16], insts: &[I]) -> Self {
        use wgpu::util::DeviceExt;
        let vertex = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
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
        let instance = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Instance Buffer"),
            contents: bytemuck::cast_slice(insts),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let instance_len = insts.len();
        Self {
            vertex,
            index,
            instance,
            index_len,
            instance_len,
            _p0: std::marker::PhantomData,
            _p1: std::marker::PhantomData,
        }
    }

    pub fn update(&self, queue: &wgpu::Queue, verts: &[V]) {
        queue.write_buffer(&self.vertex, 0, bytemuck::cast_slice(verts));
    }

    pub(crate) fn draw(&self, rpass: &mut wgpu::RenderPass) {
        rpass.set_vertex_buffer(0, self.vertex.slice(..));
        rpass.set_vertex_buffer(1, self.instance.slice(..));
        rpass.set_index_buffer(self.index.slice(..), wgpu::IndexFormat::Uint16);
        rpass.draw_indexed(0..self.index_len as u32, 0, 0..self.instance_len as u32);
    }
}

/// Instance用のVertexBuffer
pub struct VertexBufferInstanced<V> {
    pub buf: wgpu::Buffer,
    instance_len: usize,
    phantom: std::marker::PhantomData<V>,
}

impl<V> VertexBufferInstanced<V>
where
    V: bytemuck::Pod,
{
    pub fn new(device: &wgpu::Device, verts: &[V], label: Option<&str>) -> Self {
        use wgpu::util::DeviceExt;
        let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label,
            contents: bytemuck::cast_slice(verts),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        Self {
            buf,
            instance_len: verts.len(),
            phantom: std::marker::PhantomData,
        }
    }

    pub fn update(&self, queue: &wgpu::Queue, verts: &[V]) {
        queue.write_buffer(&self.buf, 0, bytemuck::cast_slice(verts));
    }

    /// 1 instanceあたりの頂点数を指定して描画
    pub fn draw(&self, rpass: &mut wgpu::RenderPass, vert_range: Range<u32>) {
        rpass.set_vertex_buffer(0, self.buf.slice(..));
        rpass.draw(vert_range, 0..self.instance_len as u32);
    }
}
