use crate::{vertex::VertexBuffer, WgpuContext};

pub mod shader;

pub struct TextureInst {
    tex: wgpu::Texture,
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
}

impl TextureInst {
    pub fn new(device: &wgpu::Device, tex: wgpu::Texture) -> Self {
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        Self { tex, view, sampler }
    }

    fn desc(&self) -> shader::bind_groups::BindGroupLayout0 {
        shader::bind_groups::BindGroupLayout0 {
            t_diffuse: &self.view,
            s_diffuse: &self.sampler,
        }
    }

    /// テクスチャにデータを書き込む
    pub fn write(&self, queue: &wgpu::Queue, data: &[u8], dim: (u32, u32), size: wgpu::Extent3d) {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * dim.0),
                rows_per_image: Some(dim.1),
            },
            size,
        );
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
        tex: &TextureInst,
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
                // データの種類は頂点毎
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

        let bind_group = shader::bind_groups::BindGroup0::from_bindings(device, tex.desc());

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
        buf: &VertexBuffer<shader::VertexInput>,
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
            buf.draw(&mut render_pass, 0..1);
        }

        state.queue().submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }
}

impl shader::VertexInput {
    pub const fn new(position: glam::Vec3, uv: glam::Vec2) -> Self {
        Self {
            position,
            tex_coords: uv,
        }
    }
}
