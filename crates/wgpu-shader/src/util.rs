use glam::Vec3;
use wgpu::TextureView;

use crate::{types, vertex::VertexBufferSimple, WgpuContext};

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

/// XY平面に1m単位のグリッドを書く
pub struct GridDrawer {
    // 補助線の色
    color_gray: glam::Vec3,
    // 主線の色の明るさ調整項目
    color_mul: glam::Vec3,
    // グリッドを展開する範囲
    range: i32,
    // 主線色にする間隔
    modulo: i32,
}

impl Default for GridDrawer {
    fn default() -> Self {
        Self {
            color_gray: Self::GLAY,
            color_mul: Vec3::ONE,
            range: 12,
            modulo: 5,
        }
    }
}

impl GridDrawer {
    const GV: f32 = 0.35;
    const GLAY: glam::Vec3 = glam::Vec3::new(Self::GV, Self::GV, Self::GV);

    /// grid頂点データ生成
    pub fn gen(&self, device: &wgpu::Device) -> VertexBufferSimple<types::vertex::Color4> {
        use glam::Vec3;
        #[allow(unused)]
        enum Axis {
            X,
            Y,
            Z,
        }

        struct Loop {
            v: Axis,
            range: (i32, i32),
            modulo: i32,
        }

        impl Loop {
            fn with_unit_x(x: i32, modulo: i32) -> Self {
                Self {
                    v: Axis::X,
                    range: (-x, x),
                    modulo,
                }
            }
            fn with_unit_y(y: i32, modulo: i32) -> Self {
                Self {
                    v: Axis::Y,
                    range: (-y, y),
                    modulo,
                }
            }

            fn unit(&self) -> Vec3 {
                match self.v {
                    Axis::X => Vec3::new(1.0, 0.0, 0.0),
                    Axis::Y => Vec3::new(0.0, 1.0, 0.0),
                    Axis::Z => Vec3::new(0.0, 0.0, 1.0),
                }
            }

            fn range(&self) -> std::ops::RangeInclusive<i32> {
                self.range.0..=self.range.1
            }

            fn min_max_vec(&self) -> (Vec3, Vec3) {
                let unit = match self.v {
                    Axis::X => Vec3::new(1.0, 0.0, 0.0),
                    Axis::Y => Vec3::new(0.0, 1.0, 0.0),
                    Axis::Z => Vec3::new(0.0, 0.0, 1.0),
                };
                let min = unit * self.range.0 as f32;
                let max = unit * self.range.1 as f32;
                (min, max)
            }

            fn range_vec(&self, color_mul: Vec3, gray: Vec3) -> impl Iterator<Item = (Vec3, Vec3)> {
                let mut v = vec![];
                for i in self.range() {
                    let color = if i % self.modulo == 0 {
                        self.unit() * color_mul
                    } else {
                        gray
                    };

                    v.push((self.unit() * i as f32, color));
                }
                v.into_iter()
            }
        }

        let (r, m) = (self.range, self.modulo);
        let mut lines = vec![];
        let loops = [
            (Loop::with_unit_x(r, m), Loop::with_unit_y(r, m)),
            (Loop::with_unit_y(r, m), Loop::with_unit_x(r, m)),
        ];
        for (fixed, moving) in loops.iter() {
            for (v, color) in moving.range_vec(self.color_mul, self.color_gray) {
                let color = color.extend(1.0);
                let (min, max) = fixed.min_max_vec();
                let min = v + min;
                let max = v + max;
                lines.push(types::vertex::Color4 {
                    position: min.extend(1.0),
                    color,
                });
                lines.push(types::vertex::Color4 {
                    position: max.extend(1.0),
                    color,
                });
            }
        }
        VertexBufferSimple::new(device, &lines, Some("Grid Lines"))
    }
}
