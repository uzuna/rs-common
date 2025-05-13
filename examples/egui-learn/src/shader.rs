use camera::{CamBufs, CamObjs, CameraBufferRequest};
use eframe::egui_wgpu;
use egui::{response, Color32};
use encase::StorageBuffer;
use fxhash::FxHashMap;
use wgpu_shader::{
    camera::FollowCamera,
    constraint::PipelineConstraint,
    model,
    prelude::{glam, Blend},
    rgltf::{PlColor, PlColorCameraBg, PlColorMaterialBg, PlColorModelBg},
    types,
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
    buffer: UniformBuffer<types::uniform::Model>,
    bg: PlColorModelBg,
    material: UniformBuffer<types::uniform::Material>,
    material_bg: PlColorMaterialBg,
}

impl DrawableResource {
    pub fn new(device: &wgpu::Device, color: glam::Vec4) -> Self {
        let buffer = types::uniform::Model::from(&glam::Mat4::IDENTITY);
        let buffer = UniformBuffer::new_encase(device, &buffer);
        let bg = PlColor::model_bg(device, buffer.buffer());

        let material = types::uniform::Material {
            color,
            ..Default::default()
        };
        let material = UniformBuffer::new_encase(device, &material);
        let material_bg = PlColor::material_bg(device, material.buffer());

        Self {
            buffer,
            bg,
            material,
            material_bg,
        }
    }

    fn update(&self, queue: &wgpu::Queue, color: glam::Vec4) {
        let material = types::uniform::Material {
            color,
            ..Default::default()
        };

        let mut byte_buffer: Vec<u8> = Vec::new();
        let mut buffer = StorageBuffer::new(&mut byte_buffer);
        buffer.write(&material).unwrap();
        queue.write_buffer(self.material.buffer(), 0, buffer.as_ref());
    }
}

/// データ更新コンテキスト
///
/// gpuリソースに関しては何も知らず、GUIからの更新で変わる可能性のある変数を保持することが期待される
pub struct Context {
    cams: CamObjs,
    color: glam::Vec4,
}

impl Context {
    pub const DEFAULT_ASPECT: f32 = 1280.0 / 720.0;
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> (Self, RenderResources) {
        let (cams, cambufs) = camera::build_cameras(device, &[1.0, Self::DEFAULT_ASPECT]);
        let p_poly = PlColor::new(
            device,
            format,
            wgpu::PrimitiveTopology::TriangleList,
            Blend::Replace,
        );
        let vert = model::cube(1.0)
            .into_iter()
            .map(|x| types::vertex::NormalColor3 {
                position: glam::Vec3::new(x.position.x, x.position.y, x.position.z),
                normal: glam::Vec3::new(1.0, 0.0, 0.0),
                color: glam::Vec3::new(x.color.x, x.color.y, x.color.z),
            })
            .collect::<Vec<_>>();
        let vb = VertexBufferSimple::new(device, &vert, None);

        let cambinds = cambufs
            .iter()
            .map(|(id, buf)| (*id, PlColor::camera_bg(device, buf)))
            .collect::<FxHashMap<u32, PlColorCameraBg>>();

        let color = glam::Vec4::new(1.0, 1.0, 1.0, 1.0);
        let dr = DrawableResource::new(device, color);
        let rr = RenderResources {
            cambufs,
            cambinds,
            p: p_poly,
            dr,
            vb,
        };

        (Self { cams, color }, rr)
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
            .show(ui, |ui: &mut egui::Ui| {
                self.paint_canvas(ui, color);
            });
        ui.label("Drag to rotate!");
    }

    fn wheel_zoom(ui: &mut egui::Ui) -> f32 {
        // wheelの回転を取得 常に1notch 40というのを勘案して調整すること
        // uiを使うということは、対象のGUI領域と無関係に行われるので、利用時には対象かどうか注意する
        ui.input(|i| i.raw_scroll_delta[1]) * -0.01
    }

    fn paint_canvas(&mut self, ui: &mut egui::Ui, color: Option<glam::Vec4>) {
        let (rect, response) =
            ui.allocate_exact_size(egui::Vec2::splat(300.0), egui::Sense::drag());
        let zoom = Self::wheel_zoom(ui);
        let cam = Self::drag_interaction(self.cams.get_mut(&0).unwrap(), response, zoom);

        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
            rect,
            FrameResources::new(0, cam.map(|x| CameraBufferRequest::new(0, x)), color),
        ));
    }

    fn drag_interaction(
        cam: &mut FollowCamera,
        response: response::Response,
        zoom: f32,
    ) -> Option<types::uniform::Camera> {
        let motion = response.drag_motion();

        if motion.x != 0.0 || motion.y != 0.0 || (zoom != 0.0 && response.contains_pointer()) {
            let yaw = response.drag_motion().x * -0.01;
            let pitch = response.drag_motion().y * -0.01;
            cam.update(pitch, yaw, zoom, false);
            Some(cam.to_uniform())
        } else {
            None
        }
    }

    pub fn shape(&mut self, ui: &mut egui::Ui) {
        egui::Frame::canvas(ui.style())
            .fill(BG_COLOR)
            .show(ui, |ui| {
                let (rect, response) =
                    ui.allocate_exact_size(egui::Vec2::new(1280.0, 720.0), egui::Sense::drag());
                let zoom = Self::wheel_zoom(ui);
                let cam = Self::drag_interaction(self.cams.get_mut(&1).unwrap(), response, zoom);
                ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                    rect,
                    FrameResources::new(1, cam.map(|x| CameraBufferRequest::new(1, x)), None),
                ));
            });
    }
}

/// レンダリング時に必要なリソースをまとめた構造体
///
/// レンダリング前、レンダリング時に固定的なリソースはここにまとめておく
pub struct RenderResources {
    cambufs: CamBufs,
    cambinds: FxHashMap<u32, PlColorCameraBg>,
    p: PlColor,
    dr: DrawableResource,
    vb: VertexBufferSimple<types::vertex::NormalColor3>,
}

impl RenderResources {
    fn prepare(&self, _device: &wgpu::Device, queue: &wgpu::Queue, f: &FrameResources) {
        if let Some(cam) = &f.cam {
            if let Some(buf) = self.cambufs.get(&cam.id) {
                queue.write_buffer(buf, 0, bytemuck::cast_slice(&[cam.cam]));
            }
        }
        if let Some(color) = &f.color {
            self.dr.update(queue, *color);
        }
    }

    fn paint(&self, render_pass: &mut wgpu::RenderPass<'static>, f: &FrameResources) {
        render_pass.set_pipeline(self.p.pipeline());
        if let Some(cambind) = self.cambinds.get(&f.cam_id) {
            cambind.set(render_pass);
        }

        self.dr.bg.set(render_pass);
        self.dr.material_bg.set(render_pass);
        self.vb.set(render_pass, 0);
        render_pass.draw(0..self.vb.len(), 0..1);
    }
}

/// レンダリング更新時にデータを配置するための型
/// 常に使い捨てる情報となっている
pub struct FrameResources {
    // レンダリングカメラセレクタ
    pub cam_id: u32,
    // 更新不要な場合は書き込みをしないためにNoneを許す
    pub cam: Option<CameraBufferRequest>,
    pub color: Option<glam::Vec4>,
}

impl FrameResources {
    pub fn new(cam_id: u32, cam: Option<CameraBufferRequest>, color: Option<glam::Vec4>) -> Self {
        Self { cam_id, cam, color }
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
        resources.paint(render_pass, self);
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

pub mod camera {
    use fxhash::FxHashMap;
    use wgpu_shader::{
        camera::{Camera, FollowCamera},
        types,
        uniform::UniformBuffer,
    };

    pub type CamObjs = FxHashMap<u32, FollowCamera>;
    pub type CamBufs = FxHashMap<u32, wgpu::Buffer>;

    pub struct CameraBufferRequest {
        pub id: u32,
        pub cam: types::uniform::Camera,
    }

    impl CameraBufferRequest {
        pub fn new(id: u32, cam: types::uniform::Camera) -> Self {
            Self { id, cam }
        }
    }

    pub fn build_cameras(device: &wgpu::Device, aspects: &[f32]) -> (CamObjs, CamBufs) {
        let mut cam = CamObjs::default();
        let mut cam_buf = CamBufs::default();
        for (i, aspect) in aspects.iter().enumerate() {
            let id = i as u32;
            let cam_obj = FollowCamera::new(Camera::with_aspect(*aspect));
            let buffer = UniformBuffer::new_encase(device, &cam_obj.camera().to_uniform());
            cam.insert(id, cam_obj);
            let buf = buffer.into_inner();
            cam_buf.insert(id, buf);
        }
        (cam, cam_buf)
    }
}
