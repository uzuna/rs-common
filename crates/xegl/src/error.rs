use khronos_egl as egl;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Egl error: {0}")]
    Egl(#[from] egl::Error),
    #[error("Failed to get display")]
    GetDisplay,
    #[error("Operation error: {0}")]
    Ops(&'static str),
}

impl Error {
    pub(crate) const fn ops(msg: &'static str) -> Self {
        Self::Ops(msg)
    }
}
