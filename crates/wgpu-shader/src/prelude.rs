pub use crate::WgpuContext;
pub use glam;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Blend {
    Replace,
    Alpha,
}
