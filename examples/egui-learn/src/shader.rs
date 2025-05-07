use eframe::egui_wgpu;
use egui::Color32;
use wgpu_shader::{
    camera::{Camera, Cams, FollowCamera},
    colored, model,
    prelude::{glam, Blend},
    types::{self, vertex::Color4},
    uniform::UniformBuffer,
    vertex::VertexBufferSimple,
};

/// eguiではRGBAを32bitでまとめて型としている
pub const BG_COLOR: Color32 = Color32::from_rgb(25, 51, 77);

/// GPUの描画に必要な最低限の情報をまとめた構造体
///
/// eguiはUIで更新情報を作り、レンダリングコンテキストで更新をする
/// もとのデータ更新をする構造体からこれらの情報を切り離して置くと扱いやすい
struct DrawableResource {
    buffer: wgpu::Buffer,
    bg: colored::DrawInfoBindGroup,
}

impl DrawableResource {
    pub fn new(device: &wgpu::Device, color: glam::Vec4) -> Self {
        let buffer = colored::unif::DrawInfo {
            matrix: glam::Mat4::IDENTITY,
            color,
        };
        let buffer = UniformBuffer::new(device, buffer);
        let bg = colored::PlUnif::make_draw_unif(device, &buffer);
        let buffer = buffer.into_inner();
        Self { buffer, bg }
    }

    fn update(&self, queue: &wgpu::Queue, color: glam::Vec4) {
        let buffer = colored::unif::DrawInfo {
            matrix: glam::Mat4::IDENTITY,
            color,
        };
        queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&[buffer]));
    }
}

/// データ更新コンテキスト
///
/// gpuリソースに関しては何も知らず、GUIからの更新で変わる可能性のある変数を保持することが期待される
pub struct Context {
    cam: FollowCamera,
    color: glam::Vec4,
}

impl Context {
    pub fn new(
        device: &wgpu::Device,
        aspect: f32,
        format: wgpu::TextureFormat,
    ) -> (Self, RenderResources) {
        let cam = Camera::with_aspect(aspect);
        let cam = Cams::new(device, cam);
        let p_poly = colored::PlUnif::new(
            device,
            format,
            cam.buffer(),
            wgpu::PrimitiveTopology::TriangleList,
            Blend::Replace,
        );
        let vb = VertexBufferSimple::new(device, &model::cube(1.0), None);
        let (cam, ub) = cam.into_inner();
        let color = glam::Vec4::new(1.0, 1.0, 1.0, 1.0);
        let dr = DrawableResource::new(device, color);
        let rr = RenderResources {
            cam: ub,
            p: p_poly,
            dr,
            vb,
        };

        (Self { cam, color }, rr)
    }

    pub fn custom_painting(&mut self, ui: &mut egui::Ui) {
        let red = &mut self.color.x;
        let res = ui.add(egui::Slider::new(red, 0.0..=1.0).text("red"));
        let color = match res.changed() {
            true => Some(self.color),
            false => None,
        };

        egui::Frame::canvas(ui.style())
            .fill(BG_COLOR)
            .show(ui, |ui| {
                self.paint_canvas(ui, color);
            });
        ui.label("Drag to rotate!");
    }

    fn paint_canvas(&mut self, ui: &mut egui::Ui, color: Option<glam::Vec4>) {
        let (rect, response) =
            ui.allocate_exact_size(egui::Vec2::splat(300.0), egui::Sense::drag());

        let motion = response.drag_motion();
        let cam = if motion.x != 0.0 || motion.y != 0.0 {
            let yaw = response.drag_motion().x * -0.01;
            let pitch = response.drag_motion().y * -0.01;
            self.cam.update(pitch, yaw, 0.0, false);
            Some(self.cam.to_uniform())
        } else {
            None
        };

        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
            rect,
            FrameResources::new(cam, color),
        ));
    }
}

/// レンダリング時に必要なリソースをまとめた構造体
///
/// レンダリング前、レンダリング時に固定的なリソースはここにまとめておく
pub struct RenderResources {
    cam: wgpu::Buffer,
    p: colored::PlUnif,
    dr: DrawableResource,
    vb: VertexBufferSimple<Color4>,
}

impl RenderResources {
    fn prepare(&self, _device: &wgpu::Device, queue: &wgpu::Queue, f: &FrameResources) {
        if let Some(cam) = &f.cam {
            queue.write_buffer(&self.cam, 0, bytemuck::cast_slice(&[*cam]));
        }
        if let Some(color) = &f.color {
            self.dr.update(queue, *color);
        }
    }

    fn paint(&self, render_pass: &mut wgpu::RenderPass<'static>) {
        self.p.set(render_pass);
        self.dr.bg.set(render_pass);
        self.vb.set(render_pass, 0);
        render_pass.draw(0..self.vb.len(), 0..1);
    }
}

/// レンダリング更新時にデータを配置するための型
/// 常に使い捨てる情報となっている
pub struct FrameResources {
    // 更新不要な場合は書き込みをしないためにNoneを許す
    pub cam: Option<types::uniform::Camera>,
    pub color: Option<glam::Vec4>,
}

impl FrameResources {
    pub fn new(cam: Option<types::uniform::Camera>, color: Option<glam::Vec4>) -> Self {
        Self { cam, color }
    }
}

impl egui_wgpu::CallbackTrait for FrameResources {
    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        // ここで方の対応付が行われている
        let resources: &RenderResources = resources.get().unwrap();
        resources.paint(render_pass);
    }

    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let resources: &RenderResources = resources.get().unwrap();
        resources.prepare(device, queue, self);
        Vec::new()
    }
}
