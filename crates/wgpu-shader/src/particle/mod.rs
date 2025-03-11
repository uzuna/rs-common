use std::ops::Range;

use encase::{internal::WriteInto, ShaderType, UniformBuffer};

use crate::WgpuContext;

pub mod shader;

impl Default for shader::VertexInput {
    fn default() -> Self {
        Self::new()
    }
}

impl shader::VertexInput {
    pub fn new() -> Self {
        Self {
            position: [0.0, 0.0, 0.0].into(),
            color: [1.0, 0.0, 0.0].into(),
        }
    }
}

pub struct Pipeline {
    pipe: wgpu::RenderPipeline,
    bind_group: shader::bind_groups::BindGroup0,
}

impl Pipeline {
    /// パイプラインの構築
    pub fn new(
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
        uniform_buffer: &Unif<shader::Window>,
    ) -> Self {
        let shader = shader::create_shader_module(device);

        let render_pipeline_layout = shader::create_pipeline_layout(device);
        let fs_target = [Some(wgpu::ColorTargetState {
            format: config.format,
            blend: Some(wgpu::BlendState {
                color: wgpu::BlendComponent::REPLACE,
                alpha: wgpu::BlendComponent::REPLACE,
            }),
            write_mask: wgpu::ColorWrites::ALL,
        })];

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: shader::vertex_state(
                &shader,
                &shader::vs_main_entry(wgpu::VertexStepMode::Vertex),
            ),
            fragment: Some(shader::fragment_state(
                &shader,
                &shader::fs_main_entry(fs_target),
            )),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::default(),
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        // uniform bufferのバインドグループの作成
        let bind_group = shader::bind_groups::BindGroup0::from_bindings(
            device,
            shader::bind_groups::BindGroupLayout0 {
                uw: uniform_buffer.buffer().as_entire_buffer_binding(),
            },
        );
        Self {
            pipe: pipeline,
            bind_group,
        }
    }

    pub fn render(
        &self,
        state: &impl WgpuContext,
        buf: &Vert<shader::VertexInput>,
    ) -> Result<(), wgpu::SurfaceError> {
        let output = state.surface().get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = state
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.1,
                            g: 0.2,
                            b: 0.3,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            render_pass.set_pipeline(&self.pipe);
            self.bind_group.set(&mut render_pass);
            // shaderが6頂点を描画する仕様になっている
            buf.draw(&mut render_pass, 0..6);
        }

        state.queue().submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }
}

pub struct Unif<U> {
    buffer: wgpu::Buffer,
    ub: UniformBuffer<Vec<u8>>,

    _phantom: std::marker::PhantomData<U>,
}

impl<U> Unif<U>
where
    U: ShaderType + WriteInto,
{
    pub fn new(device: &wgpu::Device, u: U) -> Self {
        use wgpu::util::DeviceExt;
        let mut ub = UniformBuffer::new(Vec::new());
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

    pub fn set(&mut self, queue: &wgpu::Queue, u: &U) {
        self.ub.write(u).expect("Failed to write uniform buffer");
        queue.write_buffer(&self.buffer, 0, self.ub.as_ref());
    }

    pub fn uniform_buffer(&self) -> &UniformBuffer<Vec<u8>> {
        &self.ub
    }

    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }
}

pub struct Vert<V> {
    pub buf: wgpu::Buffer,
    vert_len: usize,
    phantom: std::marker::PhantomData<V>,
}

impl<V> Vert<V>
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
            vert_len: verts.len(),
            phantom: std::marker::PhantomData,
        }
    }

    pub fn draw(&self, rpass: &mut wgpu::RenderPass, vert_range: Range<u32>) {
        rpass.set_vertex_buffer(0, self.buf.slice(..));
        rpass.draw(vert_range, 0..self.vert_len as u32);
    }
}
