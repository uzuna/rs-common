use glam::Mat4;
use wgpu::{RenderPipeline, TextureView};

use crate::{uniform::UniformBuffer, vertex::ViBuffer, WgpuContext};

pub mod light;
pub mod shader;

// #[repr(C)]
// #[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
// pub struct InstanceRaw {
//     pub model: [[f32; 4]; 4],
//     // pub normal: [[f32; 3]; 3],
// }

/// レンダリング対象の整理
///
/// 頂点とインスタンスとそのテクスチャで一つのレンダリング対象なのでまとめて扱う
pub struct Model {
    pub bg0: shader::bind_groups::BindGroup0,
    pub tex: TextureInst,
    pub vb: ViBuffer<shader::VertexInput, shader::InstanceInput>,
}

impl Model {
    pub fn new(
        device: &wgpu::Device,
        tex: TextureInst,
        vb: ViBuffer<shader::VertexInput, shader::InstanceInput>,
    ) -> Self {
        let bg0 = shader::bind_groups::BindGroup0::from_bindings(device, tex.desc());

        Self { bg0, tex, vb }
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass) {
        self.bg0.set(pass);
        self.vb.draw(pass);
    }
}

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

    pub fn desc(&self) -> shader::bind_groups::BindGroupLayout0 {
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
    pub bg1: shader::bind_groups::BindGroup1,
    pub bg2: shader::bind_groups::BindGroup2,
}

impl Pipeline {
    /// パイプラインの構築
    pub fn new(
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
        camera: &UniformBuffer<shader::Camera>,
        light: &UniformBuffer<shader::Light>,
    ) -> Self {
        let shader = shader::create_shader_module(device);

        let layout = shader::create_pipeline_layout(device);
        let fs_target = create_fs_target(config.format);
        let ve =
            shader::vs_main_entry(wgpu::VertexStepMode::Vertex, wgpu::VertexStepMode::Instance);
        let vs = shader::vertex_state(&shader, &ve);
        let fe = shader::fs_main_entry(fs_target);
        let fs = shader::fragment_state(&shader, &fe);

        let pipeline = create_render_pipeline(
            device,
            &layout,
            vs,
            Some(fs),
            Some(crate::texture::Texture::DEPTH_FORMAT),
        );

        let bg1 = shader::bind_groups::BindGroup1::from_bindings(
            device,
            shader::bind_groups::BindGroupLayout1 {
                camera: camera.buffer().as_entire_buffer_binding(),
            },
        );
        let bg2 = shader::bind_groups::BindGroup2::from_bindings(
            device,
            shader::bind_groups::BindGroupLayout2 {
                light: light.buffer().as_entire_buffer_binding(),
            },
        );

        Self {
            pipe: pipeline,
            bg1,
            bg2,
        }
    }

    pub fn pipe(&self) -> &RenderPipeline {
        &self.pipe
    }
}

impl shader::VertexInput {
    pub const fn new(position: glam::Vec3, uv: glam::Vec2) -> Self {
        Self {
            position,
            tex_coords: uv,
            normal: glam::Vec3::Z,
        }
    }
}

/// デバッグ向けの光源位置レンダリングパイプライン
pub struct LightRenderPipeline {
    pipe: wgpu::RenderPipeline,
    pub bg0: light::bind_groups::BindGroup0,
    pub bg1: light::bind_groups::BindGroup1,
}

impl LightRenderPipeline {
    /// パイプラインの構築
    pub fn new(
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
        camera: &UniformBuffer<shader::Camera>,
        light: &UniformBuffer<shader::Light>,
    ) -> Self {
        let shader = light::create_shader_module(device);

        let layout = light::create_pipeline_layout(device);
        let fs_target = create_fs_target(config.format);
        let ve = light::vs_main_entry(wgpu::VertexStepMode::Vertex);
        let vs = light::vertex_state(&shader, &ve);
        let fe = light::fs_main_entry(fs_target);
        let fs = light::fragment_state(&shader, &fe);

        let pipeline = create_render_pipeline(
            device,
            &layout,
            vs,
            Some(fs),
            Some(crate::texture::Texture::DEPTH_FORMAT),
        );

        let bg0 = light::bind_groups::BindGroup0::from_bindings(
            device,
            light::bind_groups::BindGroupLayout0 {
                camera: camera.buffer().as_entire_buffer_binding(),
            },
        );

        let bg1 = light::bind_groups::BindGroup1::from_bindings(
            device,
            light::bind_groups::BindGroupLayout1 {
                light: light.buffer().as_entire_buffer_binding(),
            },
        );

        Self {
            pipe: pipeline,
            bg0,
            bg1,
        }
    }

    pub fn pipe(&self) -> &RenderPipeline {
        &self.pipe
    }
}

// ColorTargetStateの作成の共通化
// FragmentShaderのターゲットで常に上書きをするブレンドモードを指定
fn create_fs_target(format: wgpu::TextureFormat) -> [Option<wgpu::ColorTargetState>; 1] {
    [Some(wgpu::ColorTargetState {
        format,
        blend: Some(wgpu::BlendState {
            color: wgpu::BlendComponent::REPLACE,
            alpha: wgpu::BlendComponent::REPLACE,
        }),
        write_mask: wgpu::ColorWrites::ALL,
    })]
}

// パイプライン構築の共通化
// primitiveやdepthの利用の設定などほとんどの場合共通
fn create_render_pipeline<'a>(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    vstate: wgpu::VertexState<'a>,
    fstate: Option<wgpu::FragmentState<'a>>,
    depth_format: Option<wgpu::TextureFormat>,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Render Pipeline"),
        layout: Some(layout),
        vertex: vstate,
        fragment: fstate,
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::default(),
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Back),
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        depth_stencil: depth_format.map(|format| wgpu::DepthStencilState {
            format,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: 1,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        multiview: None,
        cache: None,
    })
}

// レンダリングの共通化
pub fn render(
    state: &impl WgpuContext,
    bg_color: wgpu::Color,
    dv: &TextureView,
    f: impl FnOnce(&mut wgpu::RenderPass),
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
                    load: wgpu::LoadOp::Clear(bg_color),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: dv,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            occlusion_query_set: None,
            timestamp_writes: None,
        });

        f(&mut render_pass);
    }

    state.queue().submit(std::iter::once(encoder.finish()));
    output.present();

    Ok(())
}

pub fn create_light() -> shader::Light {
    shader::Light {
        position: glam::Vec3::new(2.0, 2.0, 2.0),
        color: glam::Vec3::new(1.0, 1.0, 1.0),
    }
}

impl From<(Mat4, Mat4)> for shader::InstanceInput {
    fn from((model, normal): (Mat4, Mat4)) -> Self {
        Self {
            model_matrix_0: model.x_axis,
            model_matrix_1: model.y_axis,
            model_matrix_2: model.z_axis,
            model_matrix_3: model.w_axis,
            normal_matrix_0: normal.x_axis,
            normal_matrix_1: normal.y_axis,
            normal_matrix_2: normal.z_axis,
            normal_matrix_3: normal.w_axis,
        }
    }
}
