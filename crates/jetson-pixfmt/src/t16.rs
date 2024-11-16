//! JetsonのT_R16をフォーマットするモジュール

use byteorder::{ByteOrder, LittleEndian};

use crate::pixfmt::CsiPixelFormat;

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

#[cfg(test)]
mod tests {
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
}
