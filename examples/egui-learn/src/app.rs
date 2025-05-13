use eframe::egui_wgpu::{self, RenderState};
use egui::Color32;
use wgpu_shader::{
    camera::{Camera, FollowCamera},
    prelude::glam,
};

use crate::{
    render::{add_camera, CameraUpdateRequest, RenderFrame, SceneResource},
    ui::move_camera_by_pointer,
};

/// eguiではRGBAを32bitでまとめて型としている
pub const BG_COLOR: Color32 = Color32::from_rgb(25, 51, 77);

/// データ更新コンテキスト
///
/// gpuリソースに関しては何も知らず、GUIからの更新で変わる可能性のある変数を保持することが期待される
pub struct Context {
    color: glam::Vec4,
    cam_id: u32,
}

impl Context {
    pub const DEFAULT_ASPECT: f32 = 1280.0 / 720.0;
    pub const CAMERA_NAME: &'static str = "main";
    pub fn new(rs: &RenderState) -> anyhow::Result<Self> {
        let cam = FollowCamera::new(Camera::with_aspect(Self::DEFAULT_ASPECT));
        let id = add_camera(rs, Self::CAMERA_NAME, cam)?;
        Ok(Self {
            color: glam::Vec4::ONE,
            cam_id: id,
        })
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

    fn paint_canvas(&mut self, ui: &mut egui::Ui, color: Option<glam::Vec4>) {
        let (rect, response) =
            ui.allocate_exact_size(egui::Vec2::splat(300.0), egui::Sense::drag());

        let prop = move_camera_by_pointer(ui, response)
            .map(|p| CameraUpdateRequest::new(SceneResource::DEFAULT_CAMERA, p));

        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
            rect,
            RenderFrame::new(prop, SceneResource::DEFAULT_CAMERA),
        ));
    }

    pub fn shape(&mut self, ui: &mut egui::Ui) {
        egui::Frame::canvas(ui.style())
            .fill(BG_COLOR)
            .show(ui, |ui| {
                let (rect, response) =
                    ui.allocate_exact_size(egui::Vec2::new(1280.0, 720.0), egui::Sense::drag());
                let prop = move_camera_by_pointer(ui, response)
                    .map(|p| CameraUpdateRequest::new(self.cam_id, p));
                ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                    rect,
                    RenderFrame::new(prop, self.cam_id),
                ));
            });
    }
}
