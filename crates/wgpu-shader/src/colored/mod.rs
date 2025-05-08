use wgpu::PrimitiveTopology;

use crate::{common, prelude::Blend, types, uniform::UniformBuffer};

#[rustfmt::skip]
pub mod instanced;
#[rustfmt::skip]
pub mod shader;
#[rustfmt::skip]
pub mod unif;

/// LinePrimitiveによる描画用のパイプライン
/// 処理負荷は少ないがデバイス依存があり演出的な表現が難しい
pub struct PipelinePrim {
    pipe: wgpu::RenderPipeline,
    bg0: shader::bind_groups::BindGroup0,
}

impl PipelinePrim {
    /// パイプラインの構築
    pub fn new(
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
        camera: &UniformBuffer<types::uniform::Camera>,
        blend: Blend,
    ) -> Self {
        let shader = shader::create_shader_module(device);

        let layout = shader::create_pipeline_layout(device);
        let fs_target = common::create_fs_target(config.format, blend);
        let ve = shader::vs_main_entry(wgpu::VertexStepMode::Vertex);
        let vs = shader::vertex_state(&shader, &ve);
        let fe = shader::fs_main_entry(fs_target);
        let fs = shader::fragment_state(&shader, &fe);

        let depth_enabled = match blend {
            Blend::Alpha => false,
            _ => true,
        };

        let pipeline = common::create_render_pipeline(
            device,
            &layout,
            vs,
            Some(fs),
            Some(crate::texture::Texture::DEPTH_FORMAT),
            PrimitiveTopology::LineList,
            depth_enabled,
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

/// 頂点とインスタンスを用いた描画用のパイプライン
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
        blend: Blend,
    ) -> Self {
        let shader = instanced::create_shader_module(device);

        let layout = instanced::create_pipeline_layout(device);
        let fs_target = common::create_fs_target(config.format, blend);
        let ve =
            instanced::vs_main_entry(wgpu::VertexStepMode::Vertex, wgpu::VertexStepMode::Instance);
        let vs = instanced::vertex_state(&shader, &ve);
        let fe = instanced::fs_main_entry(fs_target);
        let fs = instanced::fragment_state(&shader, &fe);

        let depth_enabled = match blend {
            Blend::Alpha => false,
            _ => true,
        };

        let pipeline = common::create_render_pipeline(
            device,
            &layout,
            vs,
            Some(fs),
            Some(crate::texture::Texture::DEPTH_FORMAT),
            topology,
            depth_enabled,
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

pub type DrawInfoBindGroup = unif::bind_groups::BindGroup1;
pub type CameraBindGroup = unif::bind_groups::BindGroup0;
type DrawInfoBindGroupLayout<'a> = unif::bind_groups::BindGroupLayout1<'a>;

/// SceneGraphデータ構造向けのUniformを使った値の変更を行うパイプライン
pub struct PlUnif {
    pipe: wgpu::RenderPipeline,
    // カメラはグローバルな設定で変更することはまず無いのでパイプラインと同じところで保持
    bg0: unif::bind_groups::BindGroup0,
    _topology: PrimitiveTopology,
}

impl PlUnif {
    /// パイプラインの構築
    pub fn new(
        device: &wgpu::Device,
        texture: wgpu::TextureFormat,
        camera_buf: &wgpu::Buffer,
        topology: PrimitiveTopology,
        blend: Blend,
    ) -> Self {
        use unif as s;
        let shader = s::create_shader_module(device);

        let layout = s::create_pipeline_layout(device);
        let fs_target = common::create_fs_target(texture, blend);
        let ve = s::vs_main_entry(wgpu::VertexStepMode::Vertex);
        let vs = s::vertex_state(&shader, &ve);
        let fe = s::fs_main_entry(fs_target);
        let fs = s::fragment_state(&shader, &fe);

        let depth_enabled = match blend {
            Blend::Alpha => false,
            _ => true,
        };

        let pipeline = common::create_render_pipeline(
            device,
            &layout,
            vs,
            Some(fs),
            Some(crate::texture::Texture::DEPTH_FORMAT),
            topology,
            depth_enabled,
        );

        let bg0 = s::bind_groups::BindGroup0::from_bindings(
            device,
            s::bind_groups::BindGroupLayout0 {
                camera: camera_buf.as_entire_buffer_binding(),
            },
        );

        Self {
            pipe: pipeline,
            bg0,
            _topology: topology,
        }
    }

    /// 別にカメラバインドをする場合
    pub fn make_camera_bg(device: &wgpu::Device, camera_buf: &wgpu::Buffer) -> CameraBindGroup {
        CameraBindGroup::from_bindings(
            device,
            unif::bind_groups::BindGroupLayout0 {
                camera: camera_buf.as_entire_buffer_binding(),
            },
        )
    }

    pub fn make_draw_unif(
        device: &wgpu::Device,
        object: &UniformBuffer<unif::DrawInfo>,
    ) -> DrawInfoBindGroup {
        DrawInfoBindGroup::from_bindings(
            device,
            DrawInfoBindGroupLayout {
                draw_info: object.buffer().as_entire_buffer_binding(),
            },
        )
    }

    pub fn pipeline(&self) -> &wgpu::RenderPipeline {
        &self.pipe
    }

    /// レンダリング前のバインドグループ設定など
    pub fn set(&self, pass: &mut wgpu::RenderPass) {
        pass.set_pipeline(&self.pipe);
        self.bg0.set(pass);
    }
}
