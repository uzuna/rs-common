use glam::Mat4;
use nalgebra::{Isometry3, Matrix4, Perspective3, Point3, Translation, UnitQuaternion, Vector3};

use crate::{types, uniform::UniformBuffer};

/// [nalgebra::Perspective3]の行列を、lovotの座標系に変換するための行列
/// 
/// nalgebraはOpenGLベースで-Z軸が前、Y軸が上、X軸が右、
/// lovotはROS系なので+X軸が前、Y軸が左、Z軸が上。
/// 
/// [nalgebra user guide: projections](https://nalgebra.org/docs/user_guide/projections/#perspective-projection)
#[rustfmt::skip]
pub const NALGEBRA_PERSPECTIVE_TO_MATRIX: Matrix4<f32> = Matrix4::new(
    0.0, -1.0, 0.0, 0.0,
    0.0, 0.0, 1.0, 0.0,
    -1.0, 0.0, 0.0, 0.0,
    0.0, 0.0, 0.0, 1.0,
);

/// カメラの投影行列を計算するための構造体
#[derive(Clone)]
pub struct Camera {
    /// カメラの位置
    pub eye: Point3<f32>,
    /// カメラの注視点
    pub target: Point3<f32>,
    /// カメラの上方向
    pub up: Vector3<f32>,
    /// 画面のアスペクト比。
    aspect: f32,
    /// 視野角(Yが狭く、Xはアスペクト比から計算できる)
    fovy: f32,
    /// 近距離、遠距離のものは描画対象にしないための制限
    znear: f32,
    zfar: f32,
}

impl Camera {
    /// 画面アスペクトからカメラを作成する
    ///
    /// ここではROSの座標系+X軸が前、Y軸が左、Z軸が上を想定している。
    pub fn with_aspect(aspect: f32) -> Self {
        Self::new(
            Point3::new(-3.0, 0.0, 1.7),
            Point3::new(0.0, 0.0, 0.2),
            Vector3::z(),
            aspect,
            45.0,
            0.1,
            100.0,
        )
    }

    /// カメラインスタンスの作成
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

    /// レンダラに渡す行列を取得する
    pub fn matrix(&self) -> Matrix4<f32> {
        let view = Isometry3::look_at_rh(&self.eye, &self.target, &self.up);
        let proj = Perspective3::new(self.aspect, self.fovy, self.znear, self.zfar);

        // バックエンドによっては行列が異なるのでここで行列をかける必要がある
        proj.as_matrix() * view.to_homogeneous()
    }

    /// 画面リサイズ時にアスペクト比を更新する
    pub fn set_aspect(&mut self, aspect: f32) {
        self.aspect = aspect;
    }

    /// カメラの位置を取得する
    pub fn pos(&self) -> Point3<f32> {
        self.eye
    }

    /// カメラの注視点を取得する
    pub fn target(&self) -> Point3<f32> {
        self.target
    }

    pub fn to_uniform(&self) -> types::uniform::Camera {
        let pos = self.pos();
        types::uniform::Camera {
            view_pos: glam::Vec4::new(pos.x, pos.y, pos.z, 1.0),
            view_proj: self.matrix().into(),
        }
    }
}

/// [FollowCamera]の操作に必要な情報を持つ構造体
pub struct ControlProperty {
    /// 上下方向の移動
    pub up: f32,
    /// 左右方向の移動
    pub left: f32,
    /// 前後方向の移動
    pub front: f32,
}

/// 注視点を回転中心として、距離と回転で操作をするカメラ
pub struct FollowCamera {
    // 行列計算そのものはCameraを使う
    camera: Camera,
    // resetで初期値に戻すためのバックアップフィールド
    initial: Camera,
    /// 位置を更新するために距離と回転を持つ
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
        let initial = camera.clone();
        Self {
            camera,
            initial,
            distance,
            yaw,
            pitch,
        }
    }

    /// カメラインスタンスを取得する
    pub fn camera(&self) -> &Camera {
        &self.camera
    }

    pub fn camera_mut(&mut self) -> &mut Camera {
        &mut self.camera
    }

    pub fn update_by_property(&mut self, prop: &ControlProperty, update_target: bool) {
        self.update(prop.up, prop.left, prop.front, update_target);
    }

    /// カメラ位置の更新
    ///
    /// update_targetがtrueの場合は注視点とともにカメラを平行移動させる
    pub fn update(&mut self, up_down: f32, left_right: f32, front_back: f32, update_target: bool) {
        // カメラの平行移動を行う。ある程度離れると斜めに移動することがある。TODO: 不具合解消
        if update_target {
            let q = UnitQuaternion::from_euler_angles(0.0, self.pitch, self.yaw);
            let t = self.camera.target - self.camera.eye;
            let iso = Isometry3::from_parts(Translation::from(t), q);
            let v = Vector3::new(front_back, left_right, up_down);
            let v = iso.inverse() * -v;
            self.camera.target += v;
            self.camera.eye += v;
        } else {
            // targetを中心にカメラを旋回させる
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

    /// カメラを初期位置に戻す
    pub fn reset(&mut self) {
        self.camera = self.initial.clone();
        self.distance = (self.camera.eye - self.camera.target).magnitude();
        let pitch = (self.camera.target.z - self.camera.eye.z).atan2(self.distance);
        let yaw = (self.camera.target.y - self.camera.eye.y)
            .atan2(self.camera.target.x - self.camera.eye.x)
            + std::f32::consts::PI;
        self.pitch = pitch;
        self.yaw = yaw;
    }

    pub fn to_uniform(&self) -> types::uniform::Camera {
        self.camera().to_uniform()
    }
}

pub struct Cams {
    cam: FollowCamera,
    buffer: UniformBuffer<types::uniform::Camera>,
}

impl Cams {
    pub fn new(device: &wgpu::Device, camera: Camera) -> Self {
        let cam = FollowCamera::new(camera);
        let buffer: UniformBuffer<types::uniform::Camera> =
            UniformBuffer::new(device, &cam.camera().to_uniform());
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

    /// 構造体を分解
    pub fn into_inner(self) -> (FollowCamera, wgpu::Buffer) {
        (self.cam, self.buffer.into_inner())
    }
}
