use wgpu::PrimitiveTopology;

use crate::{common, types, uniform::UniformBuffer};

#[rustfmt::skip]
pub mod instanced;
#[rustfmt::skip]
pub mod shader;

pub struct Pipeline {
    pipe: wgpu::RenderPipeline,
    bg0: shader::bind_groups::BindGroup0,
}

impl Pipeline {
    /// パイプラインの構築
    pub fn new(
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
        camera: &UniformBuffer<types::uniform::Camera>,
    ) -> Self {
        let shader = shader::create_shader_module(device);

        let layout = shader::create_pipeline_layout(device);
        let fs_target = common::create_fs_target(config.format);
        let ve = shader::vs_main_entry(wgpu::VertexStepMode::Vertex);
        let vs = shader::vertex_state(&shader, &ve);
        let fe = shader::fs_main_entry(fs_target);
        let fs = shader::fragment_state(&shader, &fe);

        let pipeline = common::create_render_pipeline(
            device,
            &layout,
            vs,
            Some(fs),
            Some(crate::texture::Texture::DEPTH_FORMAT),
            PrimitiveTopology::LineList,
        );

        let bg0 = shader::bind_groups::BindGroup0::from_bindings(
            device,
            shader::bind_groups::BindGroupLayout0 {
                camera: camera.buffer().as_entire_buffer_binding(),
            },
        );

        Self {
            pipe: pipeline,
            bg0,
        }
    }

    /// レンダリング前のバインドグループ設定など
    pub fn set(&self, pass: &mut wgpu::RenderPass) {
        pass.set_pipeline(&self.pipe);
        self.bg0.set(pass);
    }
}

pub struct PipelineInstanced {
    pipe: wgpu::RenderPipeline,
    bg0: instanced::bind_groups::BindGroup0,
    _topology: PrimitiveTopology,
}

impl PipelineInstanced {
    /// パイプラインの構築
    pub fn new(
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
        camera: &UniformBuffer<types::uniform::Camera>,
        topology: PrimitiveTopology,
    ) -> Self {
        let shader = instanced::create_shader_module(device);

        let layout = instanced::create_pipeline_layout(device);
        let fs_target = common::create_fs_target(config.format);
        let ve =
            instanced::vs_main_entry(wgpu::VertexStepMode::Vertex, wgpu::VertexStepMode::Instance);
        let vs = instanced::vertex_state(&shader, &ve);
        let fe = instanced::fs_main_entry(fs_target);
        let fs = instanced::fragment_state(&shader, &fe);

        let pipeline = common::create_render_pipeline(
            device,
            &layout,
            vs,
            Some(fs),
            Some(crate::texture::Texture::DEPTH_FORMAT),
            topology,
        );

        let bg0 = instanced::bind_groups::BindGroup0::from_bindings(
            device,
            instanced::bind_groups::BindGroupLayout0 {
                camera: camera.buffer().as_entire_buffer_binding(),
            },
        );

        Self {
            pipe: pipeline,
            bg0,
            _topology: topology,
        }
    }

    /// レンダリング前のバインドグループ設定など
    pub fn set(&self, pass: &mut wgpu::RenderPass) {
        pass.set_pipeline(&self.pipe);
        self.bg0.set(pass);
    }
}
