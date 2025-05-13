use egui::response;
use wgpu_shader::camera::ControlProperty;

/// dragとwheel操作によるカメラ移動
pub fn move_camera_by_pointer(
    ui: &mut egui::Ui,
    response: response::Response,
) -> Option<ControlProperty> {
    const CONTROL_RATE: f32 = -0.01;
    let front = ui.input(|i| i.raw_scroll_delta[1]) * CONTROL_RATE;
    let motion = response.drag_motion();

    if motion.x != 0.0 || motion.y != 0.0 || (front != 0.0 && response.contains_pointer()) {
        let left: f32 = response.drag_motion().x * CONTROL_RATE;
        let up = response.drag_motion().y * CONTROL_RATE;
        Some(ControlProperty { up, left, front })
    } else {
        None
    }
}
