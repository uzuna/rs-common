use wgpu::TextureView;

use crate::WgpuContext;

/// レンダリングの共通化
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
