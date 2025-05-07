use glam::Mat4;
use nalgebra::Matrix4;
use wgpu_shader::{
    camera::{Camera, FollowCamera},
    types,
    uniform::UniformBuffer,
};
use winit::{
    event::{ElementState, KeyEvent, WindowEvent},
    keyboard::{KeyCode, PhysicalKey},
};

#[rustfmt::skip]
pub const OPENGL_TO_WGPU_MATRIX: Matrix4<f32> = Matrix4::new(
    1.0, 0.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, 0.5, 0.5,
    0.0, 0.0, 0.0, 1.0,
);

/// CG系のZ軸前からROS系のX軸前に変換する行列
#[rustfmt::skip]
pub const ROTATION_FACE_Z_TO_X: Matrix4<f32> = Matrix4::new(
    0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, 1.0, 0.0,
    1.0, 0.0, 0.0, 0.0,
    0.0, 0.0, 0.0, 1.0,
);

/// キー入力を元にカメラを操作するコントローラー
pub struct CameraController {
    speed: f32,
    is_up_pressed: bool,
    is_down_pressed: bool,
    is_forward_pressed: bool,
    is_backward_pressed: bool,
    is_left_pressed: bool,
    is_right_pressed: bool,
}

impl CameraController {
    pub fn new(speed: f32) -> Self {
        Self {
            speed,
            is_up_pressed: false,
            is_down_pressed: false,
            is_forward_pressed: false,
            is_backward_pressed: false,
            is_left_pressed: false,
            is_right_pressed: false,
        }
    }

    /// キーイベントの読み取り
    pub fn process_events(&mut self, event: &WindowEvent) -> bool {
        match event {
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        state,
                        physical_key: PhysicalKey::Code(keycode),
                        ..
                    },
                ..
            } => {
                let is_pressed = *state == ElementState::Pressed;
                match keycode {
                    KeyCode::KeyW | KeyCode::ArrowUp => {
                        self.is_forward_pressed = is_pressed;
                        true
                    }
                    KeyCode::KeyA | KeyCode::ArrowLeft => {
                        self.is_left_pressed = is_pressed;
                        true
                    }
                    KeyCode::KeyS | KeyCode::ArrowDown => {
                        self.is_backward_pressed = is_pressed;
                        true
                    }
                    KeyCode::KeyD | KeyCode::ArrowRight => {
                        self.is_right_pressed = is_pressed;
                        true
                    }
                    KeyCode::PageUp => {
                        self.is_up_pressed = is_pressed;
                        true
                    }
                    KeyCode::PageDown => {
                        self.is_down_pressed = is_pressed;
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// カメラの位置を更新する
    pub fn update_camera(&self, camera: &mut Camera) {
        // 対象までの距離に応じてカメラの前後移動速度を変える
        let forward = camera.target - camera.eye;
        let forward_norm = forward.normalize();
        let forward_mag = forward.magnitude();

        // key状態に応じてカメラの位置を更新
        if self.is_forward_pressed && forward_mag > self.speed {
            camera.eye += forward_norm * self.speed;
        }
        if self.is_backward_pressed {
            camera.eye -= forward_norm * self.speed;
        }

        // targetを中心に視点が左右に回転する
        // カメラの移動速度が一定になる=遠くなるほど回転が遅くなるように調整している
        let right = forward_norm.cross(&camera.up);

        let forward = camera.target - camera.eye;
        let forward_mag = forward.magnitude();

        if self.is_right_pressed {
            camera.eye = camera.target - (forward + right * self.speed).normalize() * forward_mag;
        }
        if self.is_left_pressed {
            camera.eye = camera.target - (forward - right * self.speed).normalize() * forward_mag;
        }
    }

    /// target基準でカメラの位置を更新する
    pub fn update_follow_camera(&self, camera: &mut FollowCamera) {
        let up_down = if self.is_up_pressed {
            -self.speed
        } else if self.is_down_pressed {
            self.speed
        } else {
            0.0
        };
        let left_right = if self.is_left_pressed {
            -self.speed
        } else if self.is_right_pressed {
            self.speed
        } else {
            0.0
        };
        let front_back = if self.is_forward_pressed {
            -self.speed
        } else if self.is_backward_pressed {
            self.speed
        } else {
            0.0
        };
        camera.update(up_down, left_right, front_back, false);
    }
}

pub struct Cams {
    cam: FollowCamera,
    buffer: UniformBuffer<types::uniform::Camera>,
}

impl Cams {
    pub fn new(device: &wgpu::Device, camera: Camera) -> Self {
        let cam = FollowCamera::new(camera);
        let buffer = UniformBuffer::new(device, cam.camera().to_uniform());
        Self { cam, buffer }
    }

    pub fn camera_mut(&mut self) -> &mut FollowCamera {
        &mut self.cam
    }

    pub fn update(&mut self, queue: &wgpu::Queue) {
        self.buffer.write(queue, &self.cam.camera().to_uniform());
    }

    pub fn update_world(&mut self, queue: &wgpu::Queue, world: Mat4) {
        let mut cam = self.cam.camera().to_uniform();
        cam.update_world(world);
        self.buffer.write(queue, &cam);
    }

    pub fn update_world_pos(&mut self, queue: &wgpu::Queue, world: Mat4) {
        let mut cam = self.cam.camera().to_uniform();
        cam.update_world_pos(world);
        self.buffer.write(queue, &cam);
    }

    pub fn buffer(&self) -> &UniformBuffer<types::uniform::Camera> {
        &self.buffer
    }

    /// カメラオブジェクトの複製
    pub fn clone_object(&self, device: &wgpu::Device) -> Self {
        Self::new(device, self.cam.camera().clone())
    }
}
