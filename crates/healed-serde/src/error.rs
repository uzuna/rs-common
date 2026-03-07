/// ライブラリ全体で使用されるエラー型。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Invalid protection level value")]
    InvalidProtectionLevel,

    // 今後、I/Oエラーやシリアライズエラーなどを追加
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
