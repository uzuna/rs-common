use crate::{uniform::UniformBuffer, vertex::VertexBufferInstanced, WgpuContext};

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
    bg_color: wgpu::Color,
    bind_group: shader::bind_groups::BindGroup0,
}

impl Pipeline {
    /// パイプラインの構築
    pub fn new(
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
        uniform_buffer: &UniformBuffer<shader::Window>,
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
                // 内部で6頂点を描画するのでInstanceモードで描画する
                &shader::vs_main_entry(wgpu::VertexStepMode::Instance),
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
            bg_color: wgpu::Color::BLACK,
        }
    }

    pub fn set_bg_color(&mut self, color: wgpu::Color) {
        self.bg_color = color;
    }

    pub fn render(
        &self,
        state: &impl WgpuContext,
        buf: &VertexBufferInstanced<shader::VertexInput>,
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
                        load: wgpu::LoadOp::Clear(self.bg_color),
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
