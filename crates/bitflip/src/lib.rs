use std::collections::HashSet;
use std::io::{self, Write};
use std::ops::Range;
use std::time::{SystemTime, UNIX_EPOCH};

const FRAME_HEADER_BYTES: usize = 32;
const FRAME_FOOTER_BYTES: usize = 8;

/// どの領域に対してビット反転を注入するかを表します。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitFlipTarget {
    /// Primary/Secondary Header (先頭32バイト) のみを対象にします。
    HeaderOnly,
    /// ペイロード領域のみを対象にします。
    PayloadOnly,
    /// フレーム全体を対象にします。
    FullFrame,
}

impl TryFrom<&str> for BitFlipTarget {
    type Error = BitFlipError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "header_only" => Ok(Self::HeaderOnly),
            "payload_only" => Ok(Self::PayloadOnly),
            "full_frame" => Ok(Self::FullFrame),
            _ => Err(BitFlipError::InvalidTarget(value.to_string())),
        }
    }
}

/// ビット反転注入の設定です。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitFlipConfig {
    /// 反転させるビット数。
    pub flip_bits: usize,
    /// 乱数シード。`Some` の場合は再現可能、`None` の場合は非決定です。
    pub seed: Option<u64>,
    /// 反転対象領域。
    pub target: BitFlipTarget,
}

impl BitFlipConfig {
    /// 新しい設定を作成します。
    pub const fn new(flip_bits: usize, seed: Option<u64>, target: BitFlipTarget) -> Self {
        Self {
            flip_bits,
            seed,
            target,
        }
    }

    /// 注入無効（パススルー）の設定を作成します。
    pub const fn disabled() -> Self {
        Self {
            flip_bits: 0,
            seed: None,
            target: BitFlipTarget::FullFrame,
        }
    }
}

impl Default for BitFlipConfig {
    fn default() -> Self {
        Self::disabled()
    }
}

/// ビット反転注入時のエラーです。
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BitFlipError {
    /// 未知のターゲットが指定されました。
    #[error("Unknown bit flip target: {0}")]
    InvalidTarget(String),
    /// 指定されたビット数が対象領域を超えています。
    #[error("flip_bits {flip_bits} exceeds available bits {available_bits}")]
    FlipBitsOutOfRange {
        flip_bits: usize,
        available_bits: usize,
    },
}

/// `BitFlipWriter` で発生するエラーです。
#[derive(Debug, thiserror::Error)]
pub enum BitFlipWriterError {
    #[error(transparent)]
    BitFlip(#[from] BitFlipError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone, Copy)]
struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    fn new(seed: u64) -> Self {
        let normalized = if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        };
        Self { state: normalized }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn gen_index(&mut self, upper_exclusive: usize) -> usize {
        debug_assert!(upper_exclusive > 0);
        (self.next_u64() as usize) % upper_exclusive
    }
}

fn target_byte_range(total_len: usize, target: BitFlipTarget) -> Range<usize> {
    match target {
        BitFlipTarget::HeaderOnly => 0..total_len.min(FRAME_HEADER_BYTES),
        BitFlipTarget::PayloadOnly => {
            let start = total_len.min(FRAME_HEADER_BYTES);
            let end = total_len.saturating_sub(FRAME_FOOTER_BYTES);
            if end < start {
                start..start
            } else {
                start..end
            }
        }
        BitFlipTarget::FullFrame => 0..total_len,
    }
}

fn random_seed() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let salt = 0xA5A5_5A5A_0123_4567u64;
    nanos ^ nanos.rotate_left(17) ^ salt
}

fn choose_unique_offsets(total_bits: usize, flip_bits: usize, rng: &mut XorShift64) -> Vec<usize> {
    if flip_bits == 0 {
        return Vec::new();
    }

    let mut selected = HashSet::with_capacity(flip_bits);
    for j in (total_bits - flip_bits)..total_bits {
        let t = rng.gen_index(j + 1);
        if !selected.insert(t) {
            selected.insert(j);
        }
    }

    let mut offsets: Vec<usize> = selected.into_iter().collect();
    offsets.sort_unstable();
    offsets
}

/// 設定に従って、指定バッファへビット反転を注入します。
pub fn apply_bit_flip(bytes: &mut [u8], config: BitFlipConfig) -> Result<(), BitFlipError> {
    if config.flip_bits == 0 {
        return Ok(());
    }

    let range = target_byte_range(bytes.len(), config.target);
    let available_bits = range.len() * 8;
    if config.flip_bits > available_bits {
        return Err(BitFlipError::FlipBitsOutOfRange {
            flip_bits: config.flip_bits,
            available_bits,
        });
    }

    let seed = config.seed.unwrap_or_else(random_seed);
    let mut rng = XorShift64::new(seed);
    let offsets = choose_unique_offsets(available_bits, config.flip_bits, &mut rng);

    for bit_offset in offsets {
        let byte_index = range.start + (bit_offset / 8);
        let bit_index = bit_offset % 8;
        bytes[byte_index] ^= 1u8 << bit_index;
    }

    Ok(())
}

/// すべての入力を一度バッファし、`finish` 時にビット反転を注入してから書き出すWriterです。
pub struct BitFlipWriter<W: Write> {
    inner: W,
    config: BitFlipConfig,
    buffer: Vec<u8>,
}

impl<W: Write> BitFlipWriter<W> {
    /// 新しい `BitFlipWriter` を作成します。
    pub fn new(inner: W, config: BitFlipConfig) -> Self {
        Self {
            inner,
            config,
            buffer: Vec::new(),
        }
    }

    /// 現在バッファされているバイト数を返します。
    pub fn buffered_len(&self) -> usize {
        self.buffer.len()
    }

    /// バッファへビット反転を注入し、内部Writerへ書き出して返します。
    pub fn finish(mut self) -> Result<W, BitFlipWriterError> {
        apply_bit_flip(&mut self.buffer, self.config)?;
        self.inner.write_all(&self.buffer)?;
        self.inner.flush()?;
        Ok(self.inner)
    }
}

impl<W: Write> Write for BitFlipWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.buffer.extend_from_slice(buf);
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diff_bits(lhs: &[u8], rhs: &[u8]) -> usize {
        assert_eq!(lhs.len(), rhs.len(), "length mismatch in diff_bits");
        lhs.iter()
            .zip(rhs.iter())
            .map(|(l, r)| (l ^ r).count_ones() as usize)
            .sum()
    }

    fn assert_ok_flip_case(
        name: &str,
        src: &[u8],
        config: BitFlipConfig,
        expected_flip_bits: usize,
    ) {
        let mut out = src.to_vec();
        apply_bit_flip(&mut out, config)
            .unwrap_or_else(|e| panic!("{name}: unexpected error: {e}"));
        assert_eq!(
            diff_bits(src, &out),
            expected_flip_bits,
            "{name}: flipped bit count mismatch"
        );
    }

    fn assert_out_of_range_case(
        name: &str,
        src: &[u8],
        config: BitFlipConfig,
        expected_available_bits: usize,
    ) {
        let mut out = src.to_vec();
        let err = match apply_bit_flip(&mut out, config) {
            Ok(_) => panic!("{name}: expected out-of-range error"),
            Err(e) => e,
        };

        assert_eq!(
            err,
            BitFlipError::FlipBitsOutOfRange {
                flip_bits: config.flip_bits,
                available_bits: expected_available_bits,
            },
            "{name}: unexpected error"
        );
    }

    #[test]
    fn test_bitflip_value_range() {
        let src = vec![0x55u8; 5];
        let max_bits = src.len() * 8;

        let ok_cases = [
            (
                "flip_zero",
                BitFlipConfig::new(0, Some(11), BitFlipTarget::FullFrame),
                0usize,
            ),
            (
                "flip_one",
                BitFlipConfig::new(1, Some(11), BitFlipTarget::FullFrame),
                1usize,
            ),
            (
                "flip_max",
                BitFlipConfig::new(max_bits, Some(11), BitFlipTarget::FullFrame),
                max_bits,
            ),
        ];

        for (name, config, expected_flip_bits) in ok_cases {
            assert_ok_flip_case(name, &src, config, expected_flip_bits);
        }

        let err_cases = [(
            "flip_over_max",
            BitFlipConfig::new(max_bits + 1, Some(11), BitFlipTarget::FullFrame),
            max_bits,
        )];

        for (name, config, expected_available_bits) in err_cases {
            assert_out_of_range_case(name, &src, config, expected_available_bits);
        }
    }

    #[test]
    fn test_bitflip_ok_cases() {
        let full_src = vec![0xABu8; 32];
        let frame_src = vec![0xCDu8; 80];

        let flip_cases = [
            (
                "full_frame_flip",
                &full_src[..],
                BitFlipConfig::new(5, Some(1234), BitFlipTarget::FullFrame),
            ),
            (
                "header_only_flip",
                &frame_src[..],
                BitFlipConfig::new(3, Some(2222), BitFlipTarget::HeaderOnly),
            ),
            (
                "payload_only_flip",
                &frame_src[..],
                BitFlipConfig::new(7, Some(3333), BitFlipTarget::PayloadOnly),
            ),
        ];

        for (name, src, config) in flip_cases {
            assert_ok_flip_case(name, src, config, config.flip_bits);
        }

        let reproducible_cases = [
            (
                "seed_reproducible_full",
                &full_src[..],
                BitFlipConfig::new(6, Some(9001), BitFlipTarget::FullFrame),
            ),
            (
                "seed_reproducible_payload",
                &frame_src[..],
                BitFlipConfig::new(8, Some(9002), BitFlipTarget::PayloadOnly),
            ),
        ];

        for (name, src, config) in reproducible_cases {
            let mut a = src.to_vec();
            let mut b = src.to_vec();
            apply_bit_flip(&mut a, config)
                .unwrap_or_else(|e| panic!("{name}: first apply failed: {e}"));
            apply_bit_flip(&mut b, config)
                .unwrap_or_else(|e| panic!("{name}: second apply failed: {e}"));
            assert_eq!(a, b, "{name}: seed must make output reproducible");
        }

        let writer_cases = [(
            "writer_full_frame",
            &full_src[..],
            BitFlipConfig::new(4, Some(41), BitFlipTarget::FullFrame),
        )];

        for (name, src, config) in writer_cases {
            let mut writer = BitFlipWriter::new(Vec::<u8>::new(), config);
            writer
                .write_all(src)
                .unwrap_or_else(|e| panic!("{name}: write_all failed: {e}"));
            let out = writer
                .finish()
                .unwrap_or_else(|e| panic!("{name}: finish failed: {e}"));

            assert_eq!(out.len(), src.len(), "{name}: output length must be equal");
            assert_eq!(
                diff_bits(src, &out),
                config.flip_bits,
                "{name}: writer flipped bit count mismatch"
            );
        }
    }

    #[test]
    fn test_bitflip_error_cases() {
        let short_frame = vec![0xEEu8; 16];
        let tiny_frame = vec![0x12u8; 30];

        let out_of_range_cases = [
            (
                "over_full_frame",
                &short_frame[..],
                BitFlipConfig::new(129, Some(7), BitFlipTarget::FullFrame),
                128usize,
            ),
            (
                "payload_not_present",
                &tiny_frame[..],
                BitFlipConfig::new(1, Some(7), BitFlipTarget::PayloadOnly),
                0usize,
            ),
        ];

        for (name, src, config, expected_available_bits) in out_of_range_cases {
            assert_out_of_range_case(name, src, config, expected_available_bits);
        }

        let invalid_target_cases = [("invalid_target_string", "unknown_target")];

        for (name, raw_target) in invalid_target_cases {
            let err = match BitFlipTarget::try_from(raw_target) {
                Ok(_) => panic!("{name}: expected invalid target error"),
                Err(e) => e,
            };
            assert_eq!(
                err,
                BitFlipError::InvalidTarget(raw_target.to_string()),
                "{name}: invalid target error mismatch"
            );
        }
    }
}
