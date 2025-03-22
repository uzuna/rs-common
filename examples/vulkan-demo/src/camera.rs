use nalgebra::{Isometry3, Matrix4, Perspective3, Point3, Vector3};
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

pub struct Camera {
    eye: Point3<f32>,
    target: Point3<f32>,
    up: Vector3<f32>,
    aspect: f32,
    fovy: f32,
    znear: f32,
    zfar: f32,
}

impl Camera {
    /// アスペクト比を指定してよく使う設定値でカメラを作成する
    pub fn with_aspect(aspect: f32) -> Self {
        Self {
            eye: Point3::new(0.0, 1.0, 2.0),
            target: Point3::new(0.0, 0.0, 0.0),
            up: Vector3::y(),
            aspect,
            fovy: 45.0,
            znear: 0.1,
            zfar: 100.0,
        }
    }

    pub fn new(
        eye: Point3<f32>,
        target: Point3<f32>,
        up: Vector3<f32>,
        aspect: f32,
        fovy: f32,
        znear: f32,
        zfar: f32,
    ) -> Self {
        Self {
            eye,
            target,
            up,
            aspect,
            fovy,
            znear,
            zfar,
        }
    }

    pub fn build_view_projection_matrix(&self) -> Matrix4<f32> {
        // camera view matrix
        let view = Isometry3::look_at_rh(&self.eye, &self.target, &self.up);

        // camera projection matrix
        let proj = Perspective3::new(self.aspect, self.fovy, self.znear, self.zfar);

        // OpenGL uses a different coordinate system from wgpu, so we need to
        // convert the matrix.
        OPENGL_TO_WGPU_MATRIX * proj.as_matrix() * view.to_homogeneous()
    }

    /// 画面リサイズ時にアスペクト比を更新する
    pub fn set_aspect(&mut self, aspect: f32) {
        self.aspect = aspect;
    }

    pub fn pos(&self) -> Point3<f32> {
        self.eye
    }
}

/// winitに異存してカメラを操作する
pub struct CameraController {
    speed: f32,
    is_forward_pressed: bool,
    is_backward_pressed: bool,
    is_left_pressed: bool,
    is_right_pressed: bool,
}

impl CameraController {
    pub fn new(speed: f32) -> Self {
        Self {
            speed,
            is_forward_pressed: false,
            is_backward_pressed: false,
            is_left_pressed: false,
            is_right_pressed: false,
        }
    }

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
                    _ => false,
                }
            }
            _ => false,
        }
    }

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
}
