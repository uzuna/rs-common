pub mod uniform {
    /// カメラ型
    #[repr(C)]
    #[derive(Debug, Copy, Clone, PartialEq, encase::ShaderType)]
    pub struct Camera {
        pub view_pos: glam::Vec4,
        pub view_proj: glam::Mat4,
    }
}

pub mod vertex {
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
