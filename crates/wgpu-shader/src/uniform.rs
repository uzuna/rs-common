use encase::{internal::WriteInto, ShaderType, UniformBuffer as EncaseUniformBuffer};

// TODO: ubが不要なので解体する
// RAIIリソース開放するクローン不可構造体を作ることを考える
pub struct UniformBuffer<U> {
    // GPU上のメモリ位置
    buffer: wgpu::Buffer,
    // 書き込みのためのVec<u8>バッファだが、U型がencaseな場合は不要
    ub: EncaseUniformBuffer<Vec<u8>>,

    _phantom: std::marker::PhantomData<U>,
}

impl<U> UniformBuffer<U>
where
    U: ShaderType + WriteInto,
{
    fn create_buffer_init(device: &wgpu::Device, contents: &[u8]) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    /// UniformBufferの構築
    pub fn new(device: &wgpu::Device, u: U) -> Self {
        let mut ub = EncaseUniformBuffer::new(Vec::new());
        ub.write(&u).expect("Failed to write uniform buffer");
        let buffer = Self::create_buffer_init(device, ub.as_ref());
        Self {
            buffer,
            ub,
            _phantom: std::marker::PhantomData,
        }
    }

    /// UniformBufferの複製
    pub fn clone_object(&self, device: &wgpu::Device) -> Self {
        // 既存のUniformBufferの内容で新しいUniformBufferを作成
        let ub = EncaseUniformBuffer::new(self.ub.as_ref().to_vec());
        let buffer = Self::create_buffer_init(device, ub.as_ref());
        Self {
            buffer,
            ub,
            _phantom: std::marker::PhantomData,
        }
    }

    /// バッファ内容の更新
    pub fn write(&mut self, queue: &wgpu::Queue, u: &U) {
        self.ub.write(u).expect("Failed to write uniform buffer");
        queue.write_buffer(&self.buffer, 0, self.ub.as_ref());
    }

    /// GPU上のUniformBufferの参照を取得
    pub(crate) fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    pub fn into_inner(self) -> wgpu::Buffer {
        self.buffer
    }
}
