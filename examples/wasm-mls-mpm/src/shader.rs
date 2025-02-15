use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub color: [f32; 3],
}

impl Vertex {
    pub const RECT: &[Vertex; 4] = &[
        Vertex {
            position: [-0.5, -0.5, 0.0],
            color: [1.0, 0.0, 0.0],
        },
        Vertex {
            position: [0.5, -0.5, 0.0],
            color: [0.0, 1.0, 0.0],
        },
        Vertex {
            position: [0.5, 0.5, 0.0],
            color: [0.0, 0.0, 1.0],
        },
        Vertex {
            position: [-0.5, 0.5, 0.0],
            color: [1.0, 1.0, 1.0],
        },
    ];

    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x3,
                },
            ],
        }
    }
}

impl Default for Vertex {
    fn default() -> Self {
        Vertex {
            position: [0.0, 0.0, 0.0],
            color: [1.0, 0.0, 0.0],
        }
    }
}

pub struct VertexBuffer {
    pub vert: wgpu::Buffer,
    vert_len: usize,
}

impl VertexBuffer {
    pub fn new(device: &wgpu::Device, vertices: &[Vertex]) -> Self {
        let vert = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        Self {
            vert,
            vert_len: vertices.len(),
        }
    }

    pub fn update_vertices(&self, queue: &wgpu::Queue, vertices: &[Vertex]) {
        queue.write_buffer(&self.vert, 0, bytemuck::cast_slice(vertices));
    }

    pub fn draw(&self, rpass: &mut wgpu::RenderPass) {
        rpass.set_vertex_buffer(0, self.vert.slice(..));
        // 頂点の数(instanceの場合は内部生成も含む)とinstance(描画する要素数)
        // instanceは点の数なのでvertのぶんだけ。shader内で6点生成しているので6を指定
        rpass.draw(0..6, 0..self.vert_len as u32);
    }
}
