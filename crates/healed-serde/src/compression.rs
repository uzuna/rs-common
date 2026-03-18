use crate::{DataClass, Error};

const COMPRESSED_MAGIC: [u8; 4] = *b"HSZ1";
const COMPRESSED_VERSION: u8 = 1;
const CODEC_ZSTD: u8 = 1;
pub const COMPRESSED_HEADER_BYTES: usize = 14;

/// データ分類ごとの圧縮ポリシーです。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressionPolicy {
    pub small_enabled: bool,
    pub large_enabled: bool,
}

impl CompressionPolicy {
    /// 小/大サイズの両方で圧縮を有効化します。
    pub const fn enabled() -> Self {
        Self {
            small_enabled: true,
            large_enabled: true,
        }
    }

    /// 小/大サイズの両方で圧縮を無効化します。
    pub const fn disabled() -> Self {
        Self {
            small_enabled: false,
            large_enabled: false,
        }
    }

    /// 小サイズ圧縮の有効/無効を設定します。
    pub const fn with_small_enabled(mut self, enabled: bool) -> Self {
        self.small_enabled = enabled;
        self
    }

    /// 大サイズ圧縮の有効/無効を設定します。
    pub const fn with_large_enabled(mut self, enabled: bool) -> Self {
        self.large_enabled = enabled;
        self
    }

    pub const fn is_enabled_for(self, data_class: DataClass) -> bool {
        match data_class {
            DataClass::Small => self.small_enabled,
            DataClass::Large => self.large_enabled,
        }
    }
}

impl Default for CompressionPolicy {
    fn default() -> Self {
        Self::enabled()
    }
}

pub fn is_compressed_payload(bytes: &[u8]) -> bool {
    if bytes.len() < COMPRESSED_HEADER_BYTES {
        return false;
    }

    bytes[0..4] == COMPRESSED_MAGIC && bytes[4] == COMPRESSED_VERSION && bytes[5] == CODEC_ZSTD
}

pub fn maybe_compress_payload(
    payload: &[u8],
    data_class: DataClass,
    policy: CompressionPolicy,
) -> Result<Vec<u8>, Error> {
    if !policy.is_enabled_for(data_class) {
        return Ok(payload.to_vec());
    }

    #[cfg(feature = "zstd-compression")]
    {
        let compressed = zstd::stream::encode_all(payload, 0)?;
        if compressed.len() + COMPRESSED_HEADER_BYTES >= payload.len() {
            return Ok(payload.to_vec());
        }

        let mut out = Vec::with_capacity(COMPRESSED_HEADER_BYTES + compressed.len());
        out.extend_from_slice(&COMPRESSED_MAGIC);
        out.push(COMPRESSED_VERSION);
        out.push(CODEC_ZSTD);
        out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        out.extend_from_slice(&compressed);
        Ok(out)
    }

    #[cfg(not(feature = "zstd-compression"))]
    {
        Ok(payload.to_vec())
    }
}

pub fn maybe_decompress_payload(payload: &[u8]) -> Result<Vec<u8>, Error> {
    if !is_compressed_payload(payload) {
        return Ok(payload.to_vec());
    }

    let expected_len = u64::from_le_bytes(payload[6..14].try_into().unwrap()) as usize;

    #[cfg(feature = "zstd-compression")]
    {
        let body = &payload[COMPRESSED_HEADER_BYTES..];
        let decoded = zstd::stream::decode_all(body)?;
        if decoded.len() != expected_len {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "Compressed payload length mismatch: expected {expected_len}, got {}",
                    decoded.len()
                ),
            )
            .into());
        }
        Ok(decoded)
    }

    #[cfg(not(feature = "zstd-compression"))]
    {
        let _ = expected_len;
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Compressed payload requires zstd-compression feature",
        )
        .into())
    }
}
