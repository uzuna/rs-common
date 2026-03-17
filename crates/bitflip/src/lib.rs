use std::collections::HashSet;
use std::io::{self, Read, Write};
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

/// ビット反転の注入パターンです。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(Default)]
pub enum BitFlipMode {
    /// 反転位置を独立ランダムに選択します。
    #[default]
    Independent,
    /// 隣接bitを含むMBU (Multi-Bit Upset) として注入します。
    Mbu {
        /// 1つのバーストで隣接させる最大bit数です。
        max_adjacent_bits: usize,
    },
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
    /// 反転パターン。
    pub mode: BitFlipMode,
}

impl BitFlipConfig {
    /// 新しい設定を作成します。
    pub const fn new(flip_bits: usize, seed: Option<u64>, target: BitFlipTarget) -> Self {
        Self::with_mode(flip_bits, seed, target, BitFlipMode::Independent)
    }

    /// モードを含めた新しい設定を作成します。
    pub const fn with_mode(
        flip_bits: usize,
        seed: Option<u64>,
        target: BitFlipTarget,
        mode: BitFlipMode,
    ) -> Self {
        Self {
            flip_bits,
            seed,
            target,
            mode,
        }
    }

    /// MBUモードの設定を作成します。
    pub const fn mbu(
        flip_bits: usize,
        seed: Option<u64>,
        target: BitFlipTarget,
        max_adjacent_bits: usize,
    ) -> Self {
        Self::with_mode(
            flip_bits,
            seed,
            target,
            BitFlipMode::Mbu { max_adjacent_bits },
        )
    }

    /// 注入無効（パススルー）の設定を作成します。
    pub const fn disabled() -> Self {
        Self {
            flip_bits: 0,
            seed: None,
            target: BitFlipTarget::FullFrame,
            mode: BitFlipMode::Independent,
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
    /// MBUの隣接長が不正です。
    #[error("MBU max_adjacent_bits must be greater than 0")]
    InvalidMbuAdjacentBits,
}

/// `BitFlipWriter` で発生するエラーです。
#[derive(Debug, thiserror::Error)]
pub enum BitFlipWriterError {
    #[error(transparent)]
    BitFlip(#[from] BitFlipError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// TRE (Temporary Read Errors) の設定です。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreConfig {
    /// アイドル復帰直後の最初の読み出しで注入するbit数です。
    pub idle_flip_bits: usize,
    /// 乱数シード。`Some` の場合は再現可能、`None` の場合は非決定です。
    pub seed: Option<u64>,
    /// 反転対象領域。
    pub target: BitFlipTarget,
    /// 反転パターン。
    pub mode: BitFlipMode,
    /// 何回の再読み出しで完全回復するかを表します。
    pub settle_reads: usize,
}

impl TreConfig {
    /// 新しいTRE設定を作成します。
    pub const fn new(
        idle_flip_bits: usize,
        seed: Option<u64>,
        target: BitFlipTarget,
        settle_reads: usize,
    ) -> Self {
        Self::with_mode(
            idle_flip_bits,
            seed,
            target,
            BitFlipMode::Independent,
            settle_reads,
        )
    }

    /// モードを含めたTRE設定を作成します。
    pub const fn with_mode(
        idle_flip_bits: usize,
        seed: Option<u64>,
        target: BitFlipTarget,
        mode: BitFlipMode,
        settle_reads: usize,
    ) -> Self {
        Self {
            idle_flip_bits,
            seed,
            target,
            mode,
            settle_reads,
        }
    }

    /// MBUモードのTRE設定を作成します。
    pub const fn mbu(
        idle_flip_bits: usize,
        seed: Option<u64>,
        target: BitFlipTarget,
        max_adjacent_bits: usize,
        settle_reads: usize,
    ) -> Self {
        Self::with_mode(
            idle_flip_bits,
            seed,
            target,
            BitFlipMode::Mbu { max_adjacent_bits },
            settle_reads,
        )
    }

    /// TRE無効の設定を作成します。
    pub const fn disabled() -> Self {
        Self {
            idle_flip_bits: 0,
            seed: None,
            target: BitFlipTarget::FullFrame,
            mode: BitFlipMode::Independent,
            settle_reads: 1,
        }
    }
}

impl Default for TreConfig {
    fn default() -> Self {
        Self::disabled()
    }
}

/// TRE Readerの設定/構築時エラーです。
#[derive(Debug, thiserror::Error)]
pub enum TreError {
    /// 回復に必要な読み出し回数が不正です。
    #[error("settle_reads must be greater than 0 when idle_flip_bits is non-zero")]
    InvalidSettleReads,
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

fn available_bits(total_len: usize, target: BitFlipTarget) -> usize {
    target_byte_range(total_len, target).len() * 8
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

fn choose_mbu_offsets(
    total_bits: usize,
    flip_bits: usize,
    max_adjacent_bits: usize,
    rng: &mut XorShift64,
) -> Result<Vec<usize>, BitFlipError> {
    if flip_bits == 0 {
        return Ok(Vec::new());
    }

    if max_adjacent_bits == 0 {
        return Err(BitFlipError::InvalidMbuAdjacentBits);
    }

    let mut selected = vec![false; total_bits];
    let mut remaining = flip_bits;

    while remaining > 0 {
        let max_burst_len = remaining.min(max_adjacent_bits);
        let min_burst_len = if max_burst_len > 1 { 2 } else { 1 };
        let mut burst_len = if max_burst_len == min_burst_len {
            max_burst_len
        } else {
            min_burst_len + rng.gen_index(max_burst_len - min_burst_len + 1)
        };

        let mut placed = false;
        while burst_len > 0 {
            let mut candidates = Vec::new();
            let limit = total_bits.saturating_sub(burst_len) + 1;
            for start in 0..limit {
                if (start..start + burst_len).all(|index| !selected[index]) {
                    candidates.push(start);
                }
            }

            if !candidates.is_empty() {
                let start = candidates[rng.gen_index(candidates.len())];
                for index in start..start + burst_len {
                    selected[index] = true;
                }
                remaining -= burst_len;
                placed = true;
                break;
            }

            burst_len -= 1;
        }

        if !placed {
            break;
        }
    }

    let mut offsets = selected
        .into_iter()
        .enumerate()
        .filter_map(|(index, selected)| selected.then_some(index))
        .collect::<Vec<_>>();
    offsets.sort_unstable();
    Ok(offsets)
}

fn choose_offsets(
    total_bits: usize,
    flip_bits: usize,
    mode: BitFlipMode,
    rng: &mut XorShift64,
) -> Result<Vec<usize>, BitFlipError> {
    match mode {
        BitFlipMode::Independent => Ok(choose_unique_offsets(total_bits, flip_bits, rng)),
        BitFlipMode::Mbu { max_adjacent_bits } => {
            choose_mbu_offsets(total_bits, flip_bits, max_adjacent_bits, rng)
        }
    }
}

fn validate_bit_flip_config(
    total_len: usize,
    config: BitFlipConfig,
) -> Result<Range<usize>, BitFlipError> {
    if matches!(
        config.mode,
        BitFlipMode::Mbu {
            max_adjacent_bits: 0
        }
    ) && config.flip_bits > 0
    {
        return Err(BitFlipError::InvalidMbuAdjacentBits);
    }

    let range = target_byte_range(total_len, config.target);
    let available_bits = available_bits(total_len, config.target);
    if config.flip_bits > available_bits {
        return Err(BitFlipError::FlipBitsOutOfRange {
            flip_bits: config.flip_bits,
            available_bits,
        });
    }

    Ok(range)
}

fn generate_bit_offsets(
    total_len: usize,
    config: BitFlipConfig,
) -> Result<(Range<usize>, Vec<usize>), BitFlipError> {
    let range = validate_bit_flip_config(total_len, config)?;
    if config.flip_bits == 0 {
        return Ok((range, Vec::new()));
    }

    let seed = config.seed.unwrap_or_else(random_seed);
    let mut rng = XorShift64::new(seed);
    let offsets = choose_offsets(range.len() * 8, config.flip_bits, config.mode, &mut rng)?;
    Ok((range, offsets))
}

fn apply_offsets(bytes: &mut [u8], range_start: usize, offsets: &[usize]) {
    for bit_offset in offsets {
        let byte_index = range_start + (bit_offset / 8);
        let bit_index = bit_offset % 8;
        bytes[byte_index] ^= 1u8 << bit_index;
    }
}

/// 設定に従って、指定バッファへビット反転を注入します。
pub fn apply_bit_flip(bytes: &mut [u8], config: BitFlipConfig) -> Result<(), BitFlipError> {
    let (range, offsets) = generate_bit_offsets(bytes.len(), config)?;
    apply_offsets(bytes, range.start, &offsets);

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

/// TREを模擬し、アイドル復帰後の複数回読み出しで徐々に回復するReaderです。
pub struct TemporaryReadErrorReader {
    source: Vec<u8>,
    view: Vec<u8>,
    position: usize,
    config: TreConfig,
    idle_offsets: Vec<usize>,
    read_attempt: usize,
    idle_active: bool,
}

impl TemporaryReadErrorReader {
    /// バイト列から新しいTRE Readerを作成します。
    pub fn from_bytes(source: Vec<u8>, config: TreConfig) -> Result<Self, TreError> {
        validate_tre_config(source.len(), config)?;

        Ok(Self {
            view: source.clone(),
            source,
            position: 0,
            config,
            idle_offsets: Vec::new(),
            read_attempt: 0,
            idle_active: false,
        })
    }

    /// バイト列から新しいTRE Readerを作成し、直ちにアイドル復帰状態へ遷移させます。
    pub fn from_bytes_idle(source: Vec<u8>, config: TreConfig) -> Result<Self, TreError> {
        let mut reader = Self::from_bytes(source, config)?;
        reader.enter_idle();
        Ok(reader)
    }

    /// 任意のReaderを読み切って、新しいTRE Readerを作成します。
    pub fn from_reader<R: Read>(mut reader: R, config: TreConfig) -> Result<Self, TreError> {
        let mut source = Vec::new();
        reader.read_to_end(&mut source)?;
        Self::from_bytes(source, config)
    }

    /// 任意のReaderを読み切って、新しいTRE Readerを作成し、直ちにアイドル復帰状態へ遷移させます。
    pub fn from_reader_idle<R: Read>(reader: R, config: TreConfig) -> Result<Self, TreError> {
        let mut tre_reader = Self::from_reader(reader, config)?;
        tre_reader.enter_idle();
        Ok(tre_reader)
    }

    /// 元データ長を返します。
    pub fn source_len(&self) -> usize {
        self.source.len()
    }

    /// 現在有効な一時エラーbit数を返します。
    pub fn current_flip_bits(&self) -> usize {
        if !self.idle_active {
            return 0;
        }

        if self.config.settle_reads == 0 || self.read_attempt >= self.config.settle_reads {
            return 0;
        }

        let resolved_bits =
            (self.read_attempt * self.config.idle_flip_bits).div_ceil(self.config.settle_reads);
        self.config.idle_flip_bits.saturating_sub(resolved_bits)
    }

    /// アイドル復帰状態へ遷移させ、次の読み出しで一時エラーを注入します。
    pub fn enter_idle(&mut self) {
        self.position = 0;
        self.read_attempt = 0;

        if self.config.idle_flip_bits == 0 {
            self.idle_active = false;
            self.idle_offsets.clear();
            self.view.clone_from(&self.source);
            return;
        }

        let seed = self.config.seed.unwrap_or_else(random_seed);
        let bitflip_config = BitFlipConfig::with_mode(
            self.config.idle_flip_bits,
            Some(seed),
            self.config.target,
            self.config.mode,
        );
        let (_, offsets) = generate_bit_offsets(self.source.len(), bitflip_config)
            .expect("TRE config must be validated before enter_idle");

        self.idle_offsets = offsets;
        self.idle_active = true;
        self.refresh_view();
    }

    /// 読み出しを先頭からやり直し、TREの回復を1段階進めます。
    pub fn restart_read(&mut self) {
        self.position = 0;
        if self.idle_active {
            self.read_attempt += 1;
        }
        self.refresh_view();
    }

    fn refresh_view(&mut self) {
        self.view.clone_from(&self.source);

        let current_flip_bits = self.current_flip_bits();
        if current_flip_bits == 0 {
            self.idle_active = false;
            self.idle_offsets.clear();
            return;
        }

        let range = target_byte_range(self.source.len(), self.config.target);
        apply_offsets(
            &mut self.view,
            range.start,
            &self.idle_offsets[..current_flip_bits],
        );
    }
}

impl Read for TemporaryReadErrorReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let remaining = &self.view[self.position..];
        if remaining.is_empty() {
            return Ok(0);
        }

        let len = remaining.len().min(buf.len());
        buf[..len].copy_from_slice(&remaining[..len]);
        self.position += len;
        Ok(len)
    }
}

fn validate_tre_config(total_len: usize, config: TreConfig) -> Result<(), TreError> {
    if config.idle_flip_bits > 0 && config.settle_reads == 0 {
        return Err(TreError::InvalidSettleReads);
    }

    let bitflip_config = BitFlipConfig::with_mode(
        config.idle_flip_bits,
        config.seed,
        config.target,
        config.mode,
    );
    validate_bit_flip_config(total_len, bitflip_config)?;
    Ok(())
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

    fn flipped_offsets(lhs: &[u8], rhs: &[u8]) -> Vec<usize> {
        assert_eq!(lhs.len(), rhs.len(), "length mismatch in flipped_offsets");

        let mut offsets = Vec::new();
        for (byte_index, (l, r)) in lhs.iter().zip(rhs.iter()).enumerate() {
            let diff = l ^ r;
            for bit_index in 0..8 {
                if (diff & (1u8 << bit_index)) != 0 {
                    offsets.push(byte_index * 8 + bit_index);
                }
            }
        }
        offsets
    }

    fn has_adjacent_pair(offsets: &[usize]) -> bool {
        offsets.windows(2).any(|pair| pair[1] == pair[0] + 1)
    }

    fn read_all(reader: &mut TemporaryReadErrorReader) -> Vec<u8> {
        let mut out = Vec::new();
        reader.read_to_end(&mut out).unwrap();
        out
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

        let mbu_cases = [(
            "mbu_adjacent_flip",
            &frame_src[..],
            BitFlipConfig::mbu(6, Some(77), BitFlipTarget::FullFrame, 3),
        )];

        for (name, src, config) in mbu_cases {
            let mut out = src.to_vec();
            apply_bit_flip(&mut out, config)
                .unwrap_or_else(|e| panic!("{name}: apply failed: {e}"));
            let offsets = flipped_offsets(src, &out);

            assert_eq!(
                offsets.len(),
                config.flip_bits,
                "{name}: flipped bit count mismatch"
            );
            assert!(
                has_adjacent_pair(&offsets),
                "{name}: expected adjacent bit flips"
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
            (
                "invalid_mbu_adjacent_bits",
                &short_frame[..],
                BitFlipConfig::mbu(1, Some(7), BitFlipTarget::FullFrame, 0),
                short_frame.len() * 8,
            ),
        ];

        for (name, src, config, expected_available_bits) in out_of_range_cases {
            if name == "invalid_mbu_adjacent_bits" {
                let mut out = src.to_vec();
                let err = match apply_bit_flip(&mut out, config) {
                    Ok(_) => panic!("{name}: expected invalid MBU config error"),
                    Err(e) => e,
                };
                assert_eq!(
                    err,
                    BitFlipError::InvalidMbuAdjacentBits,
                    "{name}: error mismatch"
                );
                continue;
            }

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

    #[test]
    fn test_tre_value_range() {
        let src = vec![0x44u8; 64];
        let ok_cases = [(
            "tre_minimal_valid",
            TreConfig::new(1, Some(91), BitFlipTarget::FullFrame, 1),
        )];

        for (name, config) in ok_cases {
            TemporaryReadErrorReader::from_bytes(src.clone(), config)
                .unwrap_or_else(|e| panic!("{name}: expected valid TRE config: {e}"));
        }

        let err_cases = [
            (
                "tre_invalid_settle_reads",
                TreConfig::new(4, Some(91), BitFlipTarget::FullFrame, 0),
            ),
            (
                "tre_flip_over_payload",
                TreConfig::new(1000, Some(91), BitFlipTarget::PayloadOnly, 2),
            ),
        ];

        for (name, config) in err_cases {
            let err = match TemporaryReadErrorReader::from_bytes(src.clone(), config) {
                Ok(_) => panic!("{name}: expected TRE config error"),
                Err(e) => e,
            };

            match name {
                "tre_invalid_settle_reads" => {
                    assert!(
                        matches!(err, TreError::InvalidSettleReads),
                        "{name}: error mismatch"
                    );
                }
                "tre_flip_over_payload" => {
                    assert!(
                        matches!(
                            err,
                            TreError::BitFlip(BitFlipError::FlipBitsOutOfRange { .. })
                        ),
                        "{name}: unexpected error {err:?}"
                    );
                }
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn test_tre_ok_cases() {
        let src = vec![0x3Cu8; 80];
        let cases = [
            (
                "tre_recovers_over_retries",
                TreConfig::new(12, Some(501), BitFlipTarget::FullFrame, 3),
            ),
            (
                "tre_mbu_recovers_over_retries",
                TreConfig::mbu(10, Some(777), BitFlipTarget::FullFrame, 4, 2),
            ),
        ];

        for (name, config) in cases {
            let mut reader = TemporaryReadErrorReader::from_bytes_idle(src.clone(), config)
                .unwrap_or_else(|e| panic!("{name}: failed to create TRE reader: {e}"));

            let first = read_all(&mut reader);
            let first_diff = diff_bits(&src, &first);
            assert_eq!(
                first_diff, config.idle_flip_bits,
                "{name}: first read diff mismatch"
            );

            reader.restart_read();
            let second = read_all(&mut reader);
            let second_diff = diff_bits(&src, &second);
            assert!(
                second_diff <= first_diff,
                "{name}: second read should not worsen errors"
            );

            if matches!(config.mode, BitFlipMode::Mbu { .. }) {
                let offsets = flipped_offsets(&src, &first);
                assert!(
                    has_adjacent_pair(&offsets),
                    "{name}: expected adjacent TRE flips"
                );
            }

            while reader.current_flip_bits() > 0 {
                reader.restart_read();
                let _ = read_all(&mut reader);
            }

            reader.restart_read();
            let recovered = read_all(&mut reader);
            assert_eq!(
                recovered, src,
                "{name}: TRE should fully recover after retries"
            );
        }
    }

    #[test]
    fn test_tre_error_cases() {
        let payload_less_than_header = vec![0x19u8; FRAME_HEADER_BYTES - 1];
        let invalid_payload_target = TreConfig::new(1, Some(17), BitFlipTarget::PayloadOnly, 2);
        let invalid_mbu = TreConfig::mbu(4, Some(17), BitFlipTarget::FullFrame, 0, 2);

        let err_cases = [
            (
                "tre_payload_target_without_payload",
                payload_less_than_header.clone(),
                invalid_payload_target,
            ),
            ("tre_invalid_mbu", vec![0x22u8; 64], invalid_mbu),
        ];

        for (name, src, config) in err_cases {
            let err = match TemporaryReadErrorReader::from_bytes(src, config) {
                Ok(_) => panic!("{name}: expected TRE error"),
                Err(e) => e,
            };

            match name {
                "tre_payload_target_without_payload" => {
                    assert!(
                        matches!(
                            err,
                            TreError::BitFlip(BitFlipError::FlipBitsOutOfRange {
                                available_bits: 0,
                                ..
                            })
                        ),
                        "{name}: unexpected error {err:?}"
                    );
                }
                "tre_invalid_mbu" => {
                    assert!(
                        matches!(err, TreError::BitFlip(BitFlipError::InvalidMbuAdjacentBits)),
                        "{name}: error mismatch {err:?}"
                    );
                }
                _ => unreachable!(),
            }
        }
    }
}
