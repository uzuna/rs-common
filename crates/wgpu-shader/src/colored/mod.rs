use wgpu::PrimitiveTopology;

use crate::{common, types, uniform::UniformBuffer};

#[rustfmt::skip]
pub mod instanced;
#[rustfmt::skip]
pub mod shader;
#[rustfmt::skip]
pub mod compress;
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

/// 特定の座標データを圧縮、伸長するシェーダー
/// Zを0にすることでシャドウの代わりに使うことを想定している
pub struct PipelineComp {
    pipe: wgpu::RenderPipeline,
    bg0: compress::bind_groups::BindGroup0,
    bg1: compress::bind_groups::BindGroup1,
    _topology: PrimitiveTopology,
}

impl PipelineComp {
    /// パイプラインの構築
    pub fn new(
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
        camera: &UniformBuffer<types::uniform::Camera>,
        comp: &UniformBuffer<compress::Compression>,
        topology: PrimitiveTopology,
    ) -> Self {
        use compress as s;
        let shader = s::create_shader_module(device);

        let layout = s::create_pipeline_layout(device);
        let fs_target = common::create_fs_target(config.format);
        let ve = s::vs_main_entry(wgpu::VertexStepMode::Vertex, wgpu::VertexStepMode::Instance);
        let vs = s::vertex_state(&shader, &ve);
        let fe = s::fs_main_entry(fs_target);
        let fs = s::fragment_state(&shader, &fe);

        let pipeline = common::create_render_pipeline(
            device,
            &layout,
            vs,
            Some(fs),
            Some(crate::texture::Texture::DEPTH_FORMAT),
            topology,
        );

        let bg0 = s::bind_groups::BindGroup0::from_bindings(
            device,
            s::bind_groups::BindGroupLayout0 {
                camera: camera.buffer().as_entire_buffer_binding(),
            },
        );

        let bg1 = s::bind_groups::BindGroup1::from_bindings(
            device,
            s::bind_groups::BindGroupLayout1 {
                comp: comp.buffer().as_entire_buffer_binding(),
            },
        );

        Self {
            pipe: pipeline,
            bg0,
            bg1,
            _topology: topology,
        }
    }

    /// レンダリング前のバインドグループ設定など
    pub fn set(&self, pass: &mut wgpu::RenderPass) {
        pass.set_pipeline(&self.pipe);
        self.bg0.set(pass);
        self.bg1.set(pass);
    }
}

impl compress::Compression {
    pub fn xy() -> Self {
        Self {
            position: glam::Vec4::new(1.0, 1.0, 0.0, 1.0),
        }
    }
}

pub type ObjectsInfoBindGroup = unif::bind_groups::BindGroup1;
pub type DrawInfoBindGroup = unif::bind_groups::BindGroup2;

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
        config: &wgpu::SurfaceConfiguration,
        camera: &UniformBuffer<types::uniform::Camera>,
        topology: PrimitiveTopology,
    ) -> Self {
        use unif as s;
        let shader = s::create_shader_module(device);

        let layout = s::create_pipeline_layout(device);
        let fs_target = common::create_fs_target(config.format);
        let ve = s::vs_main_entry(wgpu::VertexStepMode::Vertex);
        let vs = s::vertex_state(&shader, &ve);
        let fe = s::fs_main_entry(fs_target);
        let fs = s::fragment_state(&shader, &fe);

        let pipeline = common::create_render_pipeline(
            device,
            &layout,
            vs,
            Some(fs),
            Some(crate::texture::Texture::DEPTH_FORMAT),
            topology,
        );

        let bg0 = s::bind_groups::BindGroup0::from_bindings(
            device,
            s::bind_groups::BindGroupLayout0 {
                camera: camera.buffer().as_entire_buffer_binding(),
            },
        );

        Self {
            pipe: pipeline,
            bg0,
            _topology: topology,
        }
    }

    fn make_objects_unif(
        device: &wgpu::Device,
        object: &UniformBuffer<unif::GlobalInfo>,
    ) -> unif::bind_groups::BindGroup1 {
        unif::bind_groups::BindGroup1::from_bindings(
            device,
            unif::bind_groups::BindGroupLayout1 {
                global_info: object.buffer().as_entire_buffer_binding(),
            },
        )
    }

    pub fn make_draw_unif(
        device: &wgpu::Device,
        object: &UniformBuffer<unif::DrawInfo>,
    ) -> unif::bind_groups::BindGroup2 {
        unif::bind_groups::BindGroup2::from_bindings(
            device,
            unif::bind_groups::BindGroupLayout2 {
                draw_info: object.buffer().as_entire_buffer_binding(),
            },
        )
    }

    /// レンダリング前のバインドグループ設定など
    pub fn set(&self, pass: &mut wgpu::RenderPass) {
        pass.set_pipeline(&self.pipe);
        self.bg0.set(pass);
    }
}

impl Default for unif::GlobalInfo {
    fn default() -> Self {
        Self {
            matrix: glam::Mat4::IDENTITY,
        }
    }
}

pub struct GlobalUnif {
    buffer: UniformBuffer<unif::GlobalInfo>,
    bg: unif::bind_groups::BindGroup1,
}

impl GlobalUnif {
    pub fn new(device: &wgpu::Device) -> Self {
        let buffer = UniformBuffer::new(device, unif::GlobalInfo::default());
        let bg = PlUnif::make_objects_unif(device, &buffer);
        Self { buffer, bg }
    }

    pub fn write(&mut self, queue: &wgpu::Queue, u: &unif::GlobalInfo) {
        self.buffer.write(queue, u);
    }

    pub fn bind_group(&self) -> &unif::bind_groups::BindGroup1 {
        &self.bg
    }

    pub fn set(&self, pass: &mut wgpu::RenderPass) {
        self.bg.set(pass);
    }
}
