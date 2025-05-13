use encase::StorageBuffer;

// RAIIリソース開放するクローン不可構造体を作ることを考える
pub struct UniformBuffer<U> {
    // GPU上のメモリ位置
    buffer: wgpu::Buffer,
    _phantom: std::marker::PhantomData<U>,
}

impl<U> UniformBuffer<U>
where
    U: bytemuck::NoUninit,
{
    /// アライメントが合っている構造体は直接バッファに書き込むことができる
    pub fn new(device: &wgpu::Device, u: &U) -> Self {
        let buffer = Self::create_buffer_init(device, bytemuck::cast_slice(&[*u]));
        Self {
            buffer,
            _phantom: std::marker::PhantomData,
        }
    }

    /// バッファ内容の更新
    pub fn write(&mut self, queue: &wgpu::Queue, u: &U) {
        queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&[*u]));
    }
}

impl<U> UniformBuffer<U>
where
    U: encase::ShaderType + encase::internal::WriteInto,
{
    /// WGSLと異なるアライメントの構造体は、encaseによって補正した状態なら書き込むことができる
    pub fn new_encase(device: &wgpu::Device, u: &U) -> Self {
        let mut byte_buffer: Vec<u8> = Vec::new();
        let mut buffer = StorageBuffer::new(&mut byte_buffer);
        buffer.write(&u).unwrap();
        let buffer = Self::create_buffer_init(device, buffer.as_ref());
        Self {
            buffer,
            _phantom: std::marker::PhantomData,
        }
    }

    /// バッファ内容の更新
    pub fn write_encase(&mut self, queue: &wgpu::Queue, u: &U) {
        let mut byte_buffer: Vec<u8> = Vec::new();
        let mut buffer = StorageBuffer::new(&mut byte_buffer);
        buffer.write(&u).unwrap();
        queue.write_buffer(&self.buffer, 0, buffer.as_ref());
    }
}

impl<U> UniformBuffer<U> {
    fn create_buffer_init(device: &wgpu::Device, contents: &[u8]) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    /// バッファの取得
    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    /// バッファの解体
    pub fn into_inner(self) -> wgpu::Buffer {
        self.buffer
    }
}
