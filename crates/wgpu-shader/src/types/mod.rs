pub mod uniform {
    use glam::{Vec4, Vec4Swizzles};

    use crate::graph::Trs;

    /// カメラ型
    #[repr(C)]
    #[derive(
        Debug, Copy, Clone, PartialEq, bytemuck::Pod, bytemuck::Zeroable, encase::ShaderType,
    )]
    pub struct Camera {
        pub view_pos: glam::Vec4,
        pub view_proj: glam::Mat4,
    }

    impl Camera {
        /// カメラをワールド座標に移動させる
        pub fn update_world(&mut self, world: glam::Mat4) {
            let iw = world.inverse();
            self.view_pos =
                iw * glam::Vec4::new(self.view_pos.x, self.view_pos.y, self.view_pos.z, 1.0);
            // 世界座標分元に動かしておくことで、レンダリング時には移動分がキャンセルされる
            self.view_proj *= iw;
        }

        /// カメラの位置だけ移動。床面を重視する場合はこちらのほうが良い
        pub fn update_world_pos(&mut self, world: glam::Mat4) {
            let translate = world * Vec4::new(0.0, 0.0, 0.0, 1.0);
            let matrix = glam::Mat4::from_translation(translate.xyz()).inverse();
            self.view_pos =
                matrix * glam::Vec4::new(self.view_pos.x, self.view_pos.y, self.view_pos.z, 1.0);
            self.view_proj *= matrix;
        }
    }
    /// インスタンスごとの移動とノーマル行列を持つ
    #[repr(C)]
    #[derive(Debug, Copy, Clone, PartialEq, encase::ShaderType)]
    pub struct Model {
        /// グローバルからローカル座標系への変換行列
        pub matrix: glam::Mat4,
        /// matrixに合わせた法線の変換行列
        pub normal: glam::Mat4,
    }

    impl From<&Trs> for Model {
        fn from(trs: &Trs) -> Self {
            let matrix = trs.to_homogeneous();
            let normal = matrix.inverse().transpose();
            Self { matrix, normal }
        }
    }

    /// gltf brdf(Bidirectional Reflectance Distribution Function)マテリアル情報
    #[repr(C)]
    #[derive(Debug, Copy, Clone, PartialEq, encase::ShaderType)]
    pub struct Material {
        /// ベースカラー。何もなければ白を指定する
        pub color: glam::Vec4,
        /// 金属光沢度[0.0, 1.0]
        pub metallic: f32,
        /// 表面粗さ度[0.0, 1.0]
        pub roughness: f32,
    }

    impl Material {
        const MIN: f32 = 0.0;
        const MAX: f32 = 1.0;

        pub fn set_metallic(&mut self, metallic: f32) {
            self.metallic = metallic.clamp(Self::MIN, Self::MAX);
        }

        pub fn set_roughness(&mut self, roughness: f32) {
            self.roughness = roughness.clamp(Self::MIN, Self::MAX);
        }
    }

    impl Default for Material {
        fn default() -> Self {
            Self {
                color: glam::Vec4::new(1.0, 1.0, 1.0, 1.0),
                metallic: 0.0,
                roughness: 1.0,
            }
        }
    }
}

pub mod vertex {
    use encase::ShaderType;

    /// 色付き頂点型
    #[repr(C)]
    #[derive(Debug, Copy, Clone, PartialEq, ShaderType)]
    pub struct Color3 {
        pub position: glam::Vec3,
        pub color: glam::Vec3,
    }

    impl Color3 {
        pub const fn new(position: glam::Vec3, color: glam::Vec3) -> Self {
            Self { position, color }
        }
    }

    /// 色付き頂点型
    #[repr(C)]
    #[derive(Debug, Copy, Clone, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
    pub struct Color4 {
        pub position: glam::Vec4,
        pub color: glam::Vec4,
    }

    impl Color4 {
        pub const fn new(position: glam::Vec4, color: glam::Vec4) -> Self {
            Self { position, color }
        }
    }
}

/// インスタンス向けの型定義
pub mod instance {
    /// インスタンスごとに異なるTRS(Translation, Rotation, Scale)を持つ場合の型
    #[repr(C)]
    #[derive(Debug, Copy, Clone, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
    pub struct Isometry {
        pub iso: glam::Mat4,
    }

    impl Isometry {
        pub const fn new(iso: glam::Mat4) -> Self {
            Self { iso }
        }
    }
}
