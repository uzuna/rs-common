//! JetsonのT_R16をフォーマットするモジュール

use core::slice;
use std::ops::{AddAssign, DivAssign};

use byteorder::{ByteOrder, LittleEndian};

use crate::pixfmt::CsiPixelFormat;

/// バッファの生データの保持
#[derive(Clone)]
pub struct RawBuffer {
    pub buf: Vec<u16>,
    pub format: CsiPixelFormat,
}

impl RawBuffer {
    /// 長さを指定して作成
    pub fn new(init: u16, len: usize, format: CsiPixelFormat) -> Self {
        Self {
            buf: vec![init; len],
            format,
        }
    }

    /// スライスを元に作成
    pub fn from_slice(src: &[u8], format: CsiPixelFormat) -> Self {
        let len = src.len() / 2;
        let mut buf = Vec::with_capacity(len);
        unsafe {
            std::ptr::copy(src.as_ptr() as *const u16, buf.as_mut_ptr(), len);
            buf.set_len(len);
        };
        Self { buf, format }
    }

    /// フォーマット関数を適用して作成
    pub fn with_format(
        src: &[u8],
        format: CsiPixelFormat,
        f: impl Fn(&[u8], &mut [u8], CsiPixelFormat),
    ) -> Self {
        let mut buf = vec![0_u16; src.len() / 2];
        f(
            src,
            unsafe { slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut u8, buf.len() * 2) },
            format,
        );
        Self { buf, format }
    }

    /// 取得済みバッファにデータを取り込む
    pub fn copy_from_slice(&mut self, src: &[u8]) {
        self.copy_from_slice_with_format(src, |src, buf, _format| unsafe {
            std::ptr::copy(src.as_ptr(), buf.as_mut_ptr(), src.len());
        });
    }

    /// 取得済みバッファにデータをフォーマットしながら取り込む
    pub fn copy_from_slice_with_format(
        &mut self,
        src: &[u8],
        f: impl Fn(&[u8], &mut [u8], CsiPixelFormat),
    ) {
        let buf = unsafe {
            slice::from_raw_parts_mut(self.buf.as_mut_ptr() as *mut u8, self.buf.len() * 2)
        };
        f(src, buf, self.format);
    }

    /// バッファを編集する
    pub fn modify(&mut self, f: impl Fn(&mut [u8], CsiPixelFormat)) {
        let buf = unsafe {
            slice::from_raw_parts_mut(self.buf.as_mut_ptr() as *mut u8, self.buf.len() * 2)
        };
        f(buf, self.format)
    }
}

impl AddAssign<&Self> for RawBuffer {
    fn add_assign(&mut self, rhs: &Self) {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        if std::arch::is_x86_feature_detected!("sse2") {
            return unsafe { calc::add_assign_simd(&mut self.buf, &rhs.buf) };
        }
        calc::add_assign(&mut self.buf, &rhs.buf);
    }
}

impl AddAssign<&RawSlice<'_>> for RawBuffer {
    fn add_assign(&mut self, rhs: &RawSlice) {
        let rhs = rhs.buf;
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        if std::arch::is_x86_feature_detected!("sse2") {
            return unsafe { calc::add_assign_simd(&mut self.buf, rhs) };
        }
        #[cfg(target_arch = "aarch64")]
        if std::arch::is_aarch64_feature_detected!("neon") {
            return unsafe { calc::add_assign_simd(&mut self.buf, rhs) };
        }
        calc::add_assign(&mut self.buf, rhs);
    }
}

impl DivAssign<u16> for RawBuffer {
    fn div_assign(&mut self, rhs: u16) {
        for i in self.buf.iter_mut() {
            *i /= rhs;
        }
    }
}

impl From<RawBuffer> for Vec<u8> {
    fn from(val: RawBuffer) -> Self {
        let mut buf = Vec::with_capacity(val.buf.len() * 2);
        unsafe {
            std::ptr::copy(
                val.buf.as_ptr() as *const u8,
                buf.as_mut_ptr(),
                val.buf.len() * 2,
            );
            buf.set_len(val.buf.len() * 2);
        }
        buf
    }
}

pub struct RawSlice<'d> {
    pub buf: &'d [u16],
    pub format: CsiPixelFormat,
}

impl<'d> RawSlice<'d> {
    pub fn from_slice(src: &'d [u8], format: CsiPixelFormat) -> Self {
        let len = src.len() / 2;
        let buf = unsafe { slice::from_raw_parts(src.as_ptr() as *const u16, len) };
        Self { buf, format }
    }
}

mod calc {
    //! RawBufferの計算関数

    /// u16の配列を加算
    pub fn add_assign(src: &mut [u16], rhs: &[u16]) {
        for (l, r) in src.iter_mut().zip(rhs.iter()) {
            *l += *r;
        }
    }

    /// SSE2を使ったu16の配列を加算
    #[target_feature(enable = "sse2")]
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    pub unsafe fn add_assign_simd(src: &mut [u16], rhs: &[u16]) {
        #[cfg(target_arch = "x86")]
        use std::arch::x86::*;
        #[cfg(target_arch = "x86_64")]
        use std::arch::x86_64::*;

        for i in 0..src.len() / 8 {
            let i: usize = i * 8;
            let lvec = _mm_loadu_si128(src.as_ptr().add(i) as *const _);
            let rvec = _mm_loadu_si128(rhs.as_ptr().add(i) as *const _);
            let res = _mm_add_epi16(lvec, rvec);
            _mm_storeu_si128(src.as_mut_ptr().add(i) as *mut _, res);
        }
    }

    #[target_feature(enable = "neon")]
    #[cfg(target_arch = "aarch64")]
    pub unsafe fn add_assign_simd(src: &mut [u16], rhs: &[u16]) {
        use std::arch::aarch64::*;

        #[allow(clippy::never_loop)]
        for i in 0..src.len() / 8 {
            let i: usize = i * 8;
            let lvec = vld1q_u16(src.as_ptr().add(i) as *const _);
            let rvec = vld1q_u16(rhs.as_ptr().add(i) as *const _);
            let res = vaddq_u16(lvec, rvec);
            vst1q_u16(src.as_mut_ptr().add(i) as *mut _, res);
        }
    }

    #[cfg(test)]
    mod tests {
        // テスト用のヘルパー関数
        fn assert_v(v: &[u16], expect: u16) {
            for &i in v.iter() {
                assert_eq!(i, expect);
            }
        }

        #[test]
        fn test_add_assign() {
            let mut src = vec![1; 16];
            let rhs = vec![9; 16];
            super::add_assign(&mut src, &rhs);
            assert_v(&src, 10);
        }

        #[test]
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        fn test_add_assign_simd() {
            let mut src = vec![1; 16];
            let rhs = vec![9; 16];
            unsafe { super::add_assign_simd(&mut src, &rhs) };
            assert_v(&src, 10);
        }
    }
}

/// SSE2を使って128bit幅単位でフォーマット。16byteの倍数のデータに対応
///
/// # Safety
///
/// 128bit幅単位でフォーマットするため。余った部分は変換されない。
#[target_feature(enable = "sse2")]
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
pub unsafe fn format_as_u128_simd(buf: &mut [u8], csi_format: CsiPixelFormat) {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::*;
    #[allow(overflowing_literals)]
    let shift4 = _mm_setr_epi16(csi_format.bitshift() as i16, 0, 0, 0, 0, 0, 0, 0);

    for i in 0..buf.len() / 16 {
        let i: usize = i * 16;
        let invec = _mm_loadu_si128(buf.as_ptr().add(i) as *const _);
        let shifted = _mm_srl_epi16(invec, shift4); // 論理右シフト
        _mm_storeu_si128(buf.as_mut_ptr().add(i) as *mut _, shifted);
    }
}

/// SSE2を使って128bit幅単位でフォーマット。16byteの倍数のデータに対応
///
/// # Safety
///
/// 128bit幅単位でフォーマットするため。余った部分は変換されない。
#[target_feature(enable = "sse2")]
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
pub unsafe fn format_copy_as_u128_simd(src: &[u8], dst: &mut [u8], csi_format: CsiPixelFormat) {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::*;
    #[allow(overflowing_literals)]
    let shift4 = _mm_setr_epi16(csi_format.bitshift() as i16, 0, 0, 0, 0, 0, 0, 0);

    for i in 0..src.len() / 16 {
        let i: usize = i * 16;
        let invec = _mm_loadu_si128(src.as_ptr().add(i) as *const _);
        let shifted = _mm_srl_epi16(invec, shift4); // 論理右シフト
        _mm_storeu_si128(dst.as_mut_ptr().add(i) as *mut _, shifted);
    }
}

/// AVX2を使って256bit幅単位でフォーマット。32byteの倍数のデータに対応
///
/// # Safety
///
/// 256bit幅単位でフォーマットするため。余った部分は変換されない。
#[target_feature(enable = "avx2")]
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
pub unsafe fn format_as_u256_simd(buf: &mut [u8], csi_format: CsiPixelFormat) {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::*;
    #[allow(overflowing_literals)]
    let shift4 = _mm_setr_epi16(csi_format.bitshift() as i16, 0, 0, 0, 0, 0, 0, 0);

    for i in 0..buf.len() / 32 {
        let i: usize = i * 32;
        let invec = _mm256_loadu_si256(buf.as_ptr().add(i) as *const _);
        let shifted = _mm256_srl_epi16(invec, shift4); // 論理右シフト
        _mm256_storeu_si256(buf.as_mut_ptr().add(i) as *mut _, shifted);
    }
}

/// Arm NEONを使って128bit幅単位でフォーマット
///
/// # Safety
///
/// 128bit幅単位でフォーマットするため。余った部分は変換されない。
#[target_feature(enable = "neon")]
#[cfg(target_arch = "aarch64")]
pub unsafe fn format_as_u128_simd(buf: &mut [u8], csi_format: CsiPixelFormat) {
    use std::arch::aarch64::*;
    let s = -csi_format.bitshift() as i16;
    let shift4_vec: [i16; 8] = [s, s, s, s, s, s, s, s];
    let shift4 = vld1q_s16(shift4_vec.as_ptr() as *const _);

    #[allow(clippy::never_loop)]
    for i in 0..buf.len() / 16 {
        let i: usize = i * 16;
        let invec = vld1q_u16(buf.as_ptr().add(i) as *const _);
        let res = vshlq_u16(invec, shift4);
        vst1q_u16(buf.as_mut_ptr().add(i) as *mut _, res);
    }
}

/// 128bit幅単位でフォーマット
pub fn format_as_u128(buf: &mut [u8], csi_format: CsiPixelFormat) {
    const LEN: usize = 16;
    for i in 0..buf.len() / LEN {
        let i: usize = i * LEN;
        let a = LittleEndian::read_u128(&buf[i..i + LEN]);
        LittleEndian::write_u128(&mut buf[i..i + LEN], csi_format.format_u128(a));
    }
}

/// 128bit幅単位でフォーマットとコピー
pub fn format_copy_as_u128(src: &[u8], dst: &mut [u8], csi_format: CsiPixelFormat) {
    const LEN: usize = 16;
    for i in 0..src.len() / LEN {
        let i: usize = i * LEN;
        let a = LittleEndian::read_u128(&src[i..i + LEN]);
        LittleEndian::write_u128(&mut dst[i..i + LEN], csi_format.format_u128(a));
    }
}

/// パディングを削除し、適切な数値にフォーマットする
pub fn format(buf: &mut [u8], csi_format: CsiPixelFormat) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::arch::is_x86_feature_detected!("avx2") {
            return unsafe { format_as_u256_simd(buf, csi_format) };
        } else if std::arch::is_x86_feature_detected!("sse2") {
            return unsafe { format_as_u128_simd(buf, csi_format) };
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        if std::arch::is_aarch64_feature_detected!("neon") {
            return unsafe { format_as_u128_simd(buf, csi_format) };
        }
    }
    format_as_u128(buf, csi_format);
}

pub fn format_copy(src: &[u8], dst: &mut [u8], csi_format: CsiPixelFormat) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if std::arch::is_x86_feature_detected!("sse2") {
        return unsafe { format_copy_as_u128_simd(src, dst, csi_format) };
    }
    format_copy_as_u128(src, dst, csi_format);
}

/// Paddingのみをマスクして、データが16bitの空間にマップしている結果を返す
pub fn mask_as_u128(buf: &mut [u8], csi_format: CsiPixelFormat) {
    const LEN: usize = 16;
    for i in 0..buf.len() / LEN {
        let i: usize = i * LEN;
        let a = LittleEndian::read_u128(&buf[i..i + LEN]);
        LittleEndian::write_u128(&mut buf[i..i + LEN], a & csi_format.lmask_u128());
    }
}

/// SSE2を使って128bit幅単位でマスク。16byteの倍数のデータに対応
///
/// # Safety
///
/// 128bit幅単位でマスクするため。余った部分は変換されない。
#[target_feature(enable = "sse2")]
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
pub unsafe fn mask_as_u128_simd(buf: &mut [u8], csi_format: CsiPixelFormat) {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::*;

    let mask = csi_format.lmask_u128().to_le_bytes();
    #[allow(overflowing_literals)]
    let mask = _mm_loadu_si128(mask.as_ptr() as *const _);

    for i in 0..buf.len() / 16 {
        let i: usize = i * 16;
        let invec = _mm_loadu_si128(buf.as_ptr().add(i) as *const _);
        let shifted = _mm_and_si128(invec, mask); // mask(and)演算
        _mm_storeu_si128(buf.as_mut_ptr().add(i) as *mut _, shifted);
    }
}

/// AVX2を使って256bit幅単位でマスク。32byteの倍数のデータに対応
///
/// # Safety
///
/// 256bit幅単位でマスクするため。余った部分は変換されない。
#[target_feature(enable = "avx2")]
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
pub unsafe fn mask_as_u256_simd(buf: &mut [u8], csi_format: CsiPixelFormat) {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::*;

    let mask = csi_format
        .lmask_u128()
        .to_le_bytes()
        .repeat(2)
        .into_iter()
        .collect::<Vec<u8>>();
    #[allow(overflowing_literals)]
    let mask = _mm256_loadu_si256(mask.as_ptr() as *const _);

    for i in 0..buf.len() / 32 {
        let i: usize = i * 32;
        let invec = _mm256_loadu_si256(buf.as_ptr().add(i) as *const _);
        let shifted = _mm256_and_si256(invec, mask); // 論理右シフト
        _mm256_storeu_si256(buf.as_mut_ptr().add(i) as *mut _, shifted);
    }
}

/// Arm NEONを使って128bit幅単位でマスク
///
/// # Safety
///
/// 128bit幅単位でマスクするため。余った部分は変換されない。
#[target_feature(enable = "neon")]
#[cfg(target_arch = "aarch64")]
pub unsafe fn mask_as_u128_simd(buf: &mut [u8], csi_format: CsiPixelFormat) {
    use std::arch::aarch64::*;

    let mask = csi_format.lmask_u128().to_le_bytes();
    let mask = vld1q_u16(mask.as_ptr() as *const _);

    #[allow(clippy::never_loop)]
    for i in 0..buf.len() / 16 {
        let i: usize = i * 16;
        let invec = vld1q_u16(buf.as_ptr().add(i) as *const _);
        let res = vandq_u16(invec, mask);
        vst1q_u16(buf.as_mut_ptr().add(i) as *mut _, res);
    }
}

/// 不要なデータにマスクをする。RAWをGray16bitに変換する特などに使う
pub fn mask(buf: &mut [u8], csi_format: CsiPixelFormat) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::arch::is_x86_feature_detected!("avx2") {
            return unsafe { mask_as_u256_simd(buf, csi_format) };
        } else if std::arch::is_x86_feature_detected!("sse2") {
            return unsafe { mask_as_u128_simd(buf, csi_format) };
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        if std::arch::is_aarch64_feature_detected!("neon") {
            return unsafe { mask_as_u128_simd(buf, csi_format) };
        }
    }
    mask_as_u128(buf, csi_format);
}

cfg_if::cfg_if!(
    // より小さなビット幅でのフォーマット
    // 128bit幅単位に比べるとパフォーマンスが悪いので基本使わない
    if #[cfg(feature = "as-short")] {
        /// 16bit幅単位でフォーマット
        pub fn format_as_u16(buf: &mut [u8], csi_format: CsiPixelFormat) {
            const LEN: usize = 2;
            for i in 0..buf.len() / LEN {
                let i: usize = i * LEN;
                let a = LittleEndian::read_u16(&buf[i..i + LEN]);
                LittleEndian::write_u16(&mut buf[i..i + LEN], csi_format.format_u16(a));
            }
        }

        /// 32bit幅単位でフォーマット
        pub fn format_as_u32(buf: &mut [u8], csi_format: CsiPixelFormat) {
            const LEN: usize = 4;
            for i in 0..buf.len() / LEN {
                let i: usize = i * LEN;
                let a = LittleEndian::read_u32(&buf[i..i + LEN]);
                LittleEndian::write_u32(&mut buf[i..i + 4], csi_format.format_u32(a));
            }
        }

        /// 64bit幅単位でフォーマット
        pub fn format_as_u64(buf: &mut [u8], csi_format: CsiPixelFormat) {
            const LEN: usize = 8;
            for i in 0..buf.len() / LEN {
                let i: usize = i * LEN;
                let a = LittleEndian::read_u64(&buf[i..i + LEN]);
                LittleEndian::write_u64(&mut buf[i..i + LEN], csi_format.format_u64(a));
            }
        }
    }
);

/// 128bit幅で左詰めにシフト
pub fn shift_left_as_u128(buf: &mut [u8], csi_format: CsiPixelFormat) {
    const LEN: usize = 16;
    for i in 0..buf.len() / LEN {
        let i: usize = i * LEN;
        let a = LittleEndian::read_u128(&buf[i..i + LEN]);
        LittleEndian::write_u128(&mut buf[i..i + LEN], csi_format.shift_left_u128(a));
    }
}

/// SSE2を使って128bit幅単位で左シフト
///
/// # Safety
///
/// 128bit幅単位でフォーマットするため。余った部分は変換されない。
#[target_feature(enable = "sse2")]
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
pub unsafe fn shift_left_as_u128_simd(buf: &mut [u8], csi_format: CsiPixelFormat) {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::*;
    #[allow(overflowing_literals)]
    let shift4 = _mm_setr_epi16(csi_format.bitshift() as i16, 0, 0, 0, 0, 0, 0, 0);

    for i in 0..buf.len() / 16 {
        let i: usize = i * 16;
        let invec = _mm_loadu_si128(buf.as_ptr().add(i) as *const _);
        let shifted = _mm_sll_epi16(invec, shift4); // 論理右シフト
        _mm_storeu_si128(buf.as_mut_ptr().add(i) as *mut _, shifted);
    }
}

/// Arm NEONを使って128bit幅単位で左シフト
///
/// # Safety
///
/// 128bit幅単位でフォーマットするため。余った部分は変換されない。
#[target_feature(enable = "neon")]
#[cfg(target_arch = "aarch64")]
pub unsafe fn shift_left_as_u128_simd(buf: &mut [u8], csi_format: CsiPixelFormat) {
    use std::arch::aarch64::*;
    let s = csi_format.bitshift() as i16;
    let shift4_vec: [i16; 8] = [s, s, s, s, s, s, s, s];
    let shift4 = vld1q_s16(shift4_vec.as_ptr() as *const _);

    #[allow(clippy::never_loop)]
    for i in 0..buf.len() / 16 {
        let i: usize = i * 16;
        let invec = vld1q_u16(buf.as_ptr().add(i) as *const _);
        let res = vshlq_u16(invec, shift4);
        vst1q_u16(buf.as_mut_ptr().add(i) as *mut _, res);
    }
}

/// 左詰めシフト
pub fn shift_left(buf: &mut [u8], csi_format: CsiPixelFormat) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::arch::is_x86_feature_detected!("sse2") {
            return unsafe { shift_left_as_u128_simd(buf, csi_format) };
        }
    }
    shift_left_as_u128(buf, csi_format);
}

#[cfg(test)]
mod tests {
    use std::vec;

    use super::*;

    fn to_le_bytes(data: u16, len: usize) -> Vec<u8> {
        data.to_le_bytes()
            .repeat(len)
            .into_iter()
            .collect::<Vec<u8>>()
    }

    fn format_data_raw12(len: usize) -> (Vec<u8>, Vec<u8>) {
        let buf = to_le_bytes(0xf000, len);
        let expect = to_le_bytes(0x0f00, len);
        (buf, expect)
    }

    fn mask_data(len: usize) -> (Vec<u8>, Vec<u8>) {
        let buf = to_le_bytes(0xf00f_u16, len);
        let expect = to_le_bytes(0xf000_u16, len);
        (buf, expect)
    }

    // 基本の変換
    #[test]
    fn test_format_as_u128() {
        let (mut buf, expect) = format_data_raw12(8);
        format_as_u128(&mut buf, CsiPixelFormat::Raw12);
        assert_eq!(buf, expect);
    }

    // 余りがある場合の変換
    #[test]
    fn test_format_as_u128_unaligned() {
        let (mut buf, mut expect) = format_data_raw12(12);
        // 16バイト目移行は変換されない
        let mut expect = expect.drain(..16).collect::<Vec<u8>>();
        expect.extend_from_slice(to_le_bytes(0xf000_u16, 4).as_slice());
        format_as_u128(&mut buf, CsiPixelFormat::Raw12);
        assert_eq!(buf, expect);
    }

    // x86 SSE2の変換
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    #[test]
    fn test_format_as_u128_simd() {
        let (mut buf, expect) = format_data_raw12(8);
        unsafe { format_as_u128_simd(&mut buf, CsiPixelFormat::Raw12) };
        assert_eq!(buf, expect);
    }

    // x86 AVX2の変換
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    #[test]
    fn test_format_as_u256_simd() {
        let (mut buf, expect) = format_data_raw12(16);
        unsafe { format_as_u256_simd(&mut buf, CsiPixelFormat::Raw12) };
        assert_eq!(buf, expect);
    }

    // Arm NEONの変換
    #[cfg(target_arch = "aarch64")]
    #[test]
    fn test_format_as_u128_simd_neon() {
        let (mut buf, expect) = format_data_raw12(8);
        unsafe { format_as_u128_simd(&mut buf, CsiPixelFormat::Raw12) };
        assert_eq!(buf, expect);
    }

    #[test]
    fn test_mask() {
        let (mut buf, expect) = mask_data(8);
        mask_as_u128(&mut buf, CsiPixelFormat::Raw12);
        assert_eq!(buf, expect);
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    #[test]
    fn test_mask_simd() {
        let (mut buf, expect) = mask_data(8);
        unsafe { mask_as_u128_simd(&mut buf, CsiPixelFormat::Raw12) };
        assert_eq!(buf, expect);
    }

    // Arm NEONの変換
    #[cfg(target_arch = "aarch64")]
    #[test]
    fn test_mask_as_u128_simd_neon() {
        let (mut buf, expect) = mask_data(8);
        unsafe { mask_as_u128_simd(&mut buf, CsiPixelFormat::Raw12) };
        assert_eq!(buf, expect);
    }

    #[test]
    fn test_shift_left() {
        let td = vec![(0xf800_u16, 0x8000_u16), (0x01f0, 0x1f00), (0x3084, 0x0840)];

        for (src, expect) in td {
            let mut buf = RawBuffer::new(src, 16, CsiPixelFormat::Raw12);
            buf.modify(shift_left_as_u128);
            buf.assert(expect);
            let mut buf = RawBuffer::new(src, 16, CsiPixelFormat::Raw12);
            buf.modify(|buf, csi| unsafe { shift_left_as_u128_simd(buf, csi) });
            buf.assert(expect);
        }
    }

    impl RawBuffer {
        fn assert(&self, expect: u16) {
            for b in self.buf.iter() {
                assert_eq!(*b, expect);
            }
        }
    }

    #[test]
    fn test_raw_buffer_copy_from_slice() {
        let target = 0x800f_u16;
        let mut buf = RawBuffer::new(target, 16, CsiPixelFormat::Raw12);
        buf.assert(target);

        let td = vec![0x400f_u16, 0x200f_u16, 0x100f_u16];

        for t in td {
            let n = to_le_bytes(t, 16);
            buf.copy_from_slice(n.as_slice());
            buf.assert(t);
        }
    }

    #[test]
    fn test_raw_buffer_copy_with_format() {
        let mut buf = RawBuffer::new(0, 16, CsiPixelFormat::Raw12);
        let td = vec![(0x800f_u16, 0x0800_u16), (0x1f0f, 0x01f0), (0x084f, 0x0084)];

        for (src, expect) in td {
            let n = to_le_bytes(src, 16);
            buf.copy_from_slice_with_format(n.as_slice(), format_copy_as_u128);
            buf.assert(expect);
        }
    }

    #[test]
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn test_raw_buffer_copy_with_format_simd() {
        let mut buf = RawBuffer::new(0, 16, CsiPixelFormat::Raw12);
        let td = vec![(0x800f_u16, 0x0800_u16), (0x1f0f, 0x01f0), (0x084f, 0x0084)];

        for (src, expect) in td {
            let n = to_le_bytes(src, 16);
            buf.copy_from_slice_with_format(n.as_slice(), |s, d, f| unsafe {
                format_copy_as_u128_simd(s, d, f)
            });
            buf.assert(expect);
        }
    }

    #[test]
    fn test_raw_buffer_add_assign() {
        let mut buf = RawBuffer::new(0, 16, CsiPixelFormat::Raw12);

        // Use RawBuffer
        let one = RawBuffer::new(1, 16, CsiPixelFormat::Raw12);
        for i in 1..=16 {
            buf += &one;
            buf.assert(i as u16);
        }
        buf /= 2;
        buf.assert(8);

        // Use RawSlice
        let eight = unsafe {
            #[allow(clippy::unsound_collection_transmute)]
            let mut buf = std::mem::transmute::<Vec<u16>, Vec<u8>>(vec![8_u16; 32]);
            buf.set_len(32);
            buf
        };
        let eight_slice = RawSlice::from_slice(eight.as_slice(), CsiPixelFormat::Raw12);
        buf += &eight_slice;
        buf.assert(16);
    }
}
