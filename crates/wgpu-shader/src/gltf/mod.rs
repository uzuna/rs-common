//! glTFデータに対応したシェーダー

use wgpu::PrimitiveTopology;

use crate::prelude::Blend;

#[allow(dead_code)]
mod vertex_color;

/// カメラのバインドグループ
pub type PlColorCameraBg = vertex_color::bind_groups::BindGroup0;
/// モデルのバインドグループ
pub type PlColorModelBg = vertex_color::bind_groups::BindGroup1;
/// マテリアルのバインドグループ
pub type PlColorMaterialBg = vertex_color::bind_groups::BindGroup2;

pub struct PlColor {
    pipe: wgpu::RenderPipeline,
}

impl PlColor {
    pub fn new(
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
        topology: PrimitiveTopology,
        blend: Blend,
    ) -> Self {
        use vertex_color as s;
        let shader = s::create_shader_module(device);

        let layout = s::create_pipeline_layout(device);
        let fs_target = crate::common::create_fs_target(config.format, blend);
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

    /// レンダリングパイプラインを取得
    pub fn pipeline(&self) -> &wgpu::RenderPipeline {
        &self.pipe
    }

    /// カメラバッファのバインドグループを作成
    pub fn camera_bg(device: &wgpu::Device, camera_buf: &wgpu::Buffer) -> PlColorCameraBg {
        PlColorCameraBg::from_bindings(
            device,
            vertex_color::bind_groups::BindGroupLayout0 {
                camera: camera_buf.as_entire_buffer_binding(),
            },
        )
    }

    /// モデルバッファのバインドグループを作成
    pub fn model_bg(device: &wgpu::Device, model_buf: &wgpu::Buffer) -> PlColorModelBg {
        PlColorModelBg::from_bindings(
            device,
            vertex_color::bind_groups::BindGroupLayout1 {
                model: model_buf.as_entire_buffer_binding(),
            },
        )
    }

    /// マテリアルバッファのバインドグループを作成
    pub fn material_bg(device: &wgpu::Device, material_buf: &wgpu::Buffer) -> PlColorMaterialBg {
        PlColorMaterialBg::from_bindings(
            device,
            vertex_color::bind_groups::BindGroupLayout2 {
                material: material_buf.as_entire_buffer_binding(),
            },
        )
    }
}
