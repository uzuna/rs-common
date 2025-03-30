use glam::Mat4;
use wgpu::PrimitiveTopology;

use crate::{
    common::{create_fs_target, create_render_pipeline},
    types,
    uniform::UniformBuffer,
    vertex::{InstanceBuffer, VertexBuffer},
};

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
    pub vb: VertexBuffer<shader::VertexInput>,
}

impl Model {
    pub fn new(
        device: &wgpu::Device,
        tex: TextureInst,
        vb: VertexBuffer<shader::VertexInput>,
    ) -> Self {
        let bg0 = shader::bind_groups::BindGroup0::from_bindings(device, tex.desc());

        Self { bg0, tex, vb }
    }

    pub fn set(&self, pass: &mut wgpu::RenderPass) {
        self.bg0.set(pass);
        self.vb.set(pass, 0);
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
        camera: &UniformBuffer<types::Camera>,
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
            PrimitiveTopology::default(),
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

    pub fn set(
        &self,
        pass: &mut wgpu::RenderPass,
        vb: &VertexBuffer<shader::VertexInput>,
        ib: &InstanceBuffer<shader::InstanceInput>,
    ) {
        pass.set_pipeline(&self.pipe);
        self.bg1.set(pass);
        self.bg2.set(pass);
        vb.set(pass, 0);
        ib.set(pass, 1);
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
        camera: &UniformBuffer<types::Camera>,
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
            PrimitiveTopology::TriangleList,
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

    pub fn set(&self, pass: &mut wgpu::RenderPass, vb: &VertexBuffer<shader::VertexInput>) {
        pass.set_pipeline(&self.pipe);
        self.bg0.set(pass);
        self.bg1.set(pass);
        vb.set(pass, 0);
    }
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
