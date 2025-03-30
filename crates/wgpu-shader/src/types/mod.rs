/// カメラ型
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, encase::ShaderType)]
pub struct Camera {
    pub view_pos: glam::Vec4,
    pub view_proj: glam::Mat4,
}
