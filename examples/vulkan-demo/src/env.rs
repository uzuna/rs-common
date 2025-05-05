use std::path::PathBuf;

pub struct AppEnv {
    pub assets: PathBuf,
}

impl AppEnv {
    const NAME: &'static str = "ASSETS_DIR";
    #[cfg(target_arch = "x86_64")]
    pub fn from_env() -> Self {
        let assets = PathBuf::from(
            std::env::var(Self::NAME).unwrap_or_else(|_| panic!("not dount ENV[{}]", Self::NAME)),
        );
        Self { assets }
    }
}
