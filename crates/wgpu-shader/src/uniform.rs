use encase::{internal::WriteInto, ShaderType, UniformBuffer as EncaseUniformBuffer};

pub struct UniformBuffer<U> {
    buffer: wgpu::Buffer,
    ub: EncaseUniformBuffer<Vec<u8>>,

    _phantom: std::marker::PhantomData<U>,
}

impl<U> UniformBuffer<U>
where
    U: ShaderType + WriteInto,
{
    /// UniformBufferの構築
    pub fn new(device: &wgpu::Device, u: U) -> Self {
        use wgpu::util::DeviceExt;
        let mut ub = EncaseUniformBuffer::new(Vec::new());
        ub.write(&u).expect("Failed to write uniform buffer");
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: ub.as_ref(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

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
}
