use crate::consts::{BPM_Q8_ONE, BPM_Q8_SHIFT, MS_PER_MINUTE, SIN_LUT};

#[inline]
pub const fn bpm_from_int(bpm: u16) -> u16 {
    bpm.saturating_mul(BPM_Q8_ONE)
}

#[inline]
pub const fn bpm_to_int_floor(bpm_q8: u16) -> u16 {
    bpm_q8 >> BPM_Q8_SHIFT
}

#[inline]
pub const fn bpm_to_int_round(bpm_q8: u16) -> u16 {
    ((bpm_q8 as u32 + (BPM_Q8_ONE as u32 / 2)) >> BPM_Q8_SHIFT) as u16
}

/// 入力間隔（ms）から Q8.8 BPM を求める。
#[inline]
pub fn bpm_from_interval_ms(interval_ms: u32) -> u16 {
    if interval_ms == 0 {
        return u16::MAX;
    }

    let numerator = (MS_PER_MINUTE as u64) * (BPM_Q8_ONE as u64);
    ((numerator + interval_ms as u64 / 2) / interval_ms as u64)
        .clamp(BPM_Q8_ONE as u64, u16::MAX as u64) as u16
}

/// 位相(0..65535)に対応する Q1.15 サイン値を返す。
#[inline]
pub fn fast_sin_q1_15(phase: u16) -> i16 {
    SIN_LUT[(phase >> 8) as usize]
}
