//! glTFデータに対応したシェーダー

use wgpu::PrimitiveTopology;

use crate::{
    constraint::{BindGroupImpl, PipelineConstraint},
    prelude::Blend,
};

#[allow(dead_code)]
mod vertex_color;
#[allow(dead_code)]
mod vertex_normal;

/// カメラのバインドグループ
pub type PlColorCameraBg = vertex_color::bind_groups::BindGroup0;
/// モデルのバインドグループ
pub type PlColorModelBg = vertex_color::bind_groups::BindGroup1;
/// マテリアルのバインドグループ
pub type PlColorMaterialBg = vertex_color::bind_groups::BindGroup2;

impl BindGroupImpl for PlColorCameraBg {
    fn set(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.set(pass);
    }
}
impl BindGroupImpl for PlColorModelBg {
    fn set(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.set(pass);
    }
}
impl BindGroupImpl for PlColorMaterialBg {
    fn set(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.set(pass);
    }
}

pub struct PlColor {
    pipe: wgpu::RenderPipeline,
}

impl PlColor {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        topology: PrimitiveTopology,
        blend: Blend,
    ) -> Self {
        use vertex_color as s;
        let shader = s::create_shader_module(device);

        let layout = s::create_pipeline_layout(device);
        let fs_target = crate::common::create_fs_target(format, blend);
        let ve = s::vs_main_entry(wgpu::VertexStepMode::Vertex);
        let vs = s::vertex_state(&shader, &ve);
        let fe = s::fs_main_entry(fs_target);
        let fs = s::fragment_state(&shader, &fe);

        let depth_enabled = match blend {
            Blend::Alpha => false,
            _ => true,
        };

        let pipeline = crate::common::create_render_pipeline(
            device,
            &layout,
            vs,
            Some(fs),
            Some(crate::texture::Texture::DEPTH_FORMAT),
            topology,
            depth_enabled,
        );

        Self { pipe: pipeline }
    }
}

impl PipelineConstraint for PlColor {
    type CameraBg = PlColorCameraBg;
    type ModelBg = PlColorModelBg;
    type Material = crate::types::uniform::Material;
    type MaterialBg = PlColorMaterialBg;
    type Vertex = crate::types::vertex::NormalColor3;

    fn new_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        topology: wgpu::PrimitiveTopology,
        blend: crate::prelude::Blend,
    ) -> Self {
        Self::new(device, format, topology, blend)
    }

    fn pipeline(&self) -> &wgpu::RenderPipeline {
        &self.pipe
    }

    fn camera_bg(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Self::CameraBg {
        Self::CameraBg::from_bindings(
            device,
            vertex_color::bind_groups::BindGroupLayout0 {
                camera: buffer.as_entire_buffer_binding(),
            },
        )
    }
    fn model_bg(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Self::ModelBg {
        Self::ModelBg::from_bindings(
            device,
            vertex_color::bind_groups::BindGroupLayout1 {
                model: buffer.as_entire_buffer_binding(),
            },
        )
    }
    fn material_bg(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Self::MaterialBg {
        Self::MaterialBg::from_bindings(
            device,
            vertex_color::bind_groups::BindGroupLayout2 {
                material: buffer.as_entire_buffer_binding(),
            },
        )
    }
    fn default_material() -> Self::Material {
        Self::Material::default()
    }
}

/// カメラのバインドグループ
pub type PlNormalCameraBg = vertex_normal::bind_groups::BindGroup0;
/// モデルのバインドグループ
pub type PlNormalModelBg = vertex_normal::bind_groups::BindGroup1;
/// マテリアルのバインドグループ
pub type PlNormalMaterialBg = vertex_normal::bind_groups::BindGroup2;

impl BindGroupImpl for PlNormalCameraBg {
    fn set(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.set(pass);
    }
}
impl BindGroupImpl for PlNormalModelBg {
    fn set(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.set(pass);
    }
}
impl BindGroupImpl for PlNormalMaterialBg {
    fn set(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.set(pass);
    }
}

pub struct PlNormal {
    pipe: wgpu::RenderPipeline,
}

impl PlNormal {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        topology: PrimitiveTopology,
        blend: Blend,
    ) -> Self {
        use vertex_normal as s;
        let shader = s::create_shader_module(device);

        let layout = s::create_pipeline_layout(device);
        let fs_target = crate::common::create_fs_target(format, blend);
        let ve = s::vs_main_entry(wgpu::VertexStepMode::Vertex);
        let vs = s::vertex_state(&shader, &ve);
        let fe = s::fs_main_entry(fs_target);
        let fs = s::fragment_state(&shader, &fe);

        let depth_enabled = match blend {
            Blend::Alpha => false,
            _ => true,
        };

        let pipeline = crate::common::create_render_pipeline(
            device,
            &layout,
            vs,
            Some(fs),
            Some(crate::texture::Texture::DEPTH_FORMAT),
            topology,
            depth_enabled,
        );

        Self { pipe: pipeline }
    }
}

impl PipelineConstraint for PlNormal {
    type CameraBg = PlNormalCameraBg;
    type ModelBg = PlNormalModelBg;
    type Material = crate::types::uniform::Material;
    type MaterialBg = PlNormalMaterialBg;
    type Vertex = crate::types::vertex::Normal;

    fn new_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        topology: wgpu::PrimitiveTopology,
        blend: crate::prelude::Blend,
    ) -> Self {
        Self::new(device, format, topology, blend)
    }

    fn pipeline(&self) -> &wgpu::RenderPipeline {
        &self.pipe
    }

    fn camera_bg(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Self::CameraBg {
        Self::CameraBg::from_bindings(
            device,
            vertex_normal::bind_groups::BindGroupLayout0 {
                camera: buffer.as_entire_buffer_binding(),
            },
        )
    }
    fn model_bg(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Self::ModelBg {
        Self::ModelBg::from_bindings(
            device,
            vertex_normal::bind_groups::BindGroupLayout1 {
                model: buffer.as_entire_buffer_binding(),
            },
        )
    }
    fn material_bg(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Self::MaterialBg {
        Self::MaterialBg::from_bindings(
            device,
            vertex_normal::bind_groups::BindGroupLayout2 {
                material: buffer.as_entire_buffer_binding(),
            },
        )
    }
    fn default_material() -> Self::Material {
        Self::Material::default()
    }
}
