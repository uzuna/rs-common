use glam::Vec4;
use nalgebra::{Isometry3, Matrix4, Perspective3, Point3, UnitQuaternion, Vector3};
use wgpu_shader::{types, uniform::UniformBuffer};
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
            eye: Point3::new(-3.0, 0.0, 1.7),
            target: Point3::new(0.0, 0.0, 0.2),
            up: Vector3::z(),
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

    pub fn build_buffer(&self) -> types::uniform::Camera {
        let view_proj = self.build_view_projection_matrix().into();
        let view_pos = Vec4::new(self.eye.x, self.eye.y, self.eye.z, 1.0);
        types::uniform::Camera {
            view_pos,
            view_proj,
        }
    }
}

/// targetを中心に行って距離と周囲の移動を行うカメラ
/// XY平面でZをUpとするカメラ
pub struct FollowCamera {
    camera: Camera,
    distance: f32,
    yaw: f32,
    pitch: f32,
}

impl FollowCamera {
    /// 真上と真下の回転を避けるために、pitchの範囲を制限する
    const PITCH_MARGIN: f32 = 0.001;
    pub fn new(camera: Camera) -> Self {
        let distance = (camera.eye - camera.target).magnitude();
        let pitch = (camera.target.z - camera.eye.z).atan2(distance);
        let yaw = (camera.target.y - camera.eye.y).atan2(camera.target.x - camera.eye.x)
            + std::f32::consts::PI;
        Self {
            camera,
            distance,
            yaw,
            pitch,
        }
    }

    pub fn camera(&self) -> &Camera {
        &self.camera
    }

    pub fn camera_mut(&mut self) -> &mut Camera {
        &mut self.camera
    }

    pub fn update(&mut self, up_down: f32, left_right: f32, front_back: f32) {
        use std::f32::consts::FRAC_PI_2;
        self.distance += front_back;
        self.yaw += left_right;
        self.pitch += up_down;
        self.pitch = self
            .pitch
            .clamp(-FRAC_PI_2 + Self::PITCH_MARGIN, Self::PITCH_MARGIN);
        let q = UnitQuaternion::from_euler_angles(0.0, self.pitch, self.yaw);
        let point = self.camera.target + q * Vector3::new(self.distance, 0.0, 0.0);
        self.camera.eye = point;
    }
}

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
        camera.update(up_down, left_right, front_back);
    }
}

pub struct Cams {
    cam: FollowCamera,
    buffer: UniformBuffer<types::uniform::Camera>,
}

impl Cams {
    pub fn new(device: &wgpu::Device, camera: Camera) -> Self {
        let cam = FollowCamera::new(camera);
        let buffer = UniformBuffer::new(device, cam.camera().build_buffer());
        Self { cam, buffer }
    }

    pub fn camera_mut(&mut self) -> &mut FollowCamera {
        &mut self.cam
    }

    pub fn update(&mut self, queue: &wgpu::Queue) {
        self.buffer.write(queue, &self.cam.camera().build_buffer());
    }

    pub fn buffer(&self) -> &UniformBuffer<types::uniform::Camera> {
        &self.buffer
    }
}
