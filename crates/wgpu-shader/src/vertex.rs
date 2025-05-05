/// 頂点のみで構成するVertexBuffer
/// 修飾のないlineやpointの描画に使うことを想定
pub struct VertexBufferSimple<V> {
    buf: wgpu::Buffer,
    vertex_len: u32,
    phantom: std::marker::PhantomData<V>,
}
impl<V> VertexBufferSimple<V>
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
            vertex_len: verts.len() as u32,
            phantom: std::marker::PhantomData,
        }
    }

    /// 頂点バッファの更新
    pub fn update(&self, queue: &wgpu::Queue, verts: &[V]) {
        queue.write_buffer(&self.buf, 0, bytemuck::cast_slice(verts));
    }

    /// レンダリングパスにバッファをセット
    pub fn set(&self, rpass: &mut wgpu::RenderPass, slot: u32) {
        rpass.set_vertex_buffer(slot, self.buf.slice(..));
    }

    /// 頂点数を取得。描画は[wgpu::RenderPass::draw]を使う
    pub fn len(&self) -> u32 {
        self.vertex_len
    }

    /// レンダリングオブジェクト向けのバインディングインスタンス
    pub fn bind_buffer(&self, slot: u32, topology: Topology) -> VbBinding {
        VbBinding::new(self.buf.clone(), slot, topology, self.vertex_len)
    }
}

/// 頂点とインデックスで構成するVertexBuffer
/// ポリゴンを用いた描画に使うことを想定
pub struct VertexBuffer<V> {
    buf: wgpu::Buffer,
    index: wgpu::Buffer,
    index_len: u32,
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
        let index_len = indexes.len() as u32;
        Self {
            buf,
            index,
            index_len,
            phantom: std::marker::PhantomData,
        }
    }

    /// 頂点バッファの更新
    pub fn update(&self, queue: &wgpu::Queue, verts: &[V]) {
        queue.write_buffer(&self.buf, 0, bytemuck::cast_slice(verts));
    }

    /// レンダリングパスにバッファをセット
    pub fn set(&self, rpass: &mut wgpu::RenderPass, slot: u32) {
        rpass.set_vertex_buffer(slot, self.buf.slice(..));
        rpass.set_index_buffer(self.index.slice(..), wgpu::IndexFormat::Uint16);
    }

    /// インデックスバッファの更新。描画は[wgpu::RenderPass::draw_indexed]を使う
    pub fn len(&self) -> u32 {
        self.index_len
    }
}

/// インスタンスバッファ
///
/// 複雑な頂点データをを持つデータを複数描画する場合に使う
pub struct InstanceBuffer<I> {
    buf: wgpu::Buffer,
    instance_len: u32,
    phantom: std::marker::PhantomData<I>,
}

impl<I> InstanceBuffer<I>
where
    I: bytemuck::Pod,
{
    pub fn new(device: &wgpu::Device, insts: &[I]) -> Self {
        use wgpu::util::DeviceExt;
        let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Instance Buffer"),
            contents: bytemuck::cast_slice(insts),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let instance_len = insts.len() as u32;
        Self {
            buf,
            instance_len,
            phantom: std::marker::PhantomData,
        }
    }

    /// インスタンスバッファの更新
    pub fn update(&self, queue: &wgpu::Queue, insts: &[I]) {
        queue.write_buffer(&self.buf, 0, bytemuck::cast_slice(insts));
    }

    /// レンダリングパスにバッファをセット
    pub fn set(&self, rpass: &mut wgpu::RenderPass, slot: u32) {
        rpass.set_vertex_buffer(slot, self.buf.slice(..));
    }

    /// インスタンス数を取得。描画時のinstance rengeに対して`0..self.len()`を指定する
    pub fn len(&self) -> u32 {
        self.instance_len
    }
}

/// ノードグラフのオブジェクトが頂点への参照を持つための構造体
#[derive(Clone)]
pub struct VbBinding {
    // GPUBufferへのポインタ
    vb: wgpu::Buffer,
    // 頂点データの長さ。sideではなく頂点データの数
    len: u32,
    // レンダリングパスでのバインド位置
    slot: u32,
    // レンダリングパスに期待するトポロジ
    topology: Topology,
}

impl VbBinding {
    fn new(vb: wgpu::Buffer, slot: u32, topology: Topology, len: u32) -> Self {
        Self {
            vb,
            slot,
            topology,
            len,
        }
    }

    pub fn topology(&self) -> Topology {
        self.topology
    }

    pub fn set(&self, rpass: &mut wgpu::RenderPass) {
        rpass.set_vertex_buffer(self.slot, self.vb.slice(..));
    }

    pub fn len(&self) -> u32 {
        self.len
    }
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord)]
pub enum Topology {
    LineList,
    TriangleList,
}
