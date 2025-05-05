use crate::prelude::Blend;

// ColorTargetStateの作成の共通化
// FragmentShaderのターゲットで常に上書きをするブレンドモードを指定
pub fn create_fs_target(
    format: wgpu::TextureFormat,
    blend: Blend,
) -> [Option<wgpu::ColorTargetState>; 1] {
    let blend = match blend {
        Blend::Replace => wgpu::BlendState {
            color: wgpu::BlendComponent::REPLACE,
            alpha: wgpu::BlendComponent::REPLACE,
        },
        Blend::Alpha => wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        },
    };
    [Some(wgpu::ColorTargetState {
        format,
        blend: Some(blend),
        write_mask: wgpu::ColorWrites::ALL,
    })]
}

// パイプライン構築の共通化
// primitiveやdepthの利用の設定などほとんどの場合共通
pub fn create_render_pipeline<'a>(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    vstate: wgpu::VertexState<'a>,
    fstate: Option<wgpu::FragmentState<'a>>,
    depth_format: Option<wgpu::TextureFormat>,
    topology: wgpu::PrimitiveTopology,
    depth_write_enabled: bool,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Render Pipeline"),
        layout: Some(layout),
        vertex: vstate,
        fragment: fstate,
        primitive: wgpu::PrimitiveState {
            topology,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Back),
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        depth_stencil: depth_format.map(|format| wgpu::DepthStencilState {
            format,
            depth_write_enabled,
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
