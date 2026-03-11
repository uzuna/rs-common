use serde::{Deserialize, Serialize};

pub mod comm;
pub mod consts;
pub mod fixed_math;

use consts::RHYTHM_MESSAGE_WIRE_SIZE;
use fixed_math::{phase_advance_sub16, BpmQ8, PhaseU16};

const PULSE_INTERVAL_HISTORY_SIZE: usize = 8;
const EMA_Q8_ONE: u32 = 256;

// 下流クレートが使う公開定数・ユーティリティを再エクスポートする。
pub use consts::{BPM_Q8_ONE, BPM_Q8_SHIFT, MS_PER_MINUTE, PHASE_MODULUS};
pub use fixed_math::BpmLimitParam;

/// `sync_pulse` 振る舞いを制御するパラメータ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PulseSyncParam {
    /// 想定拍でパルスが来ない状態が何周期続いたら `WaitSecondPoint` へ戻すか。
    pub missing_cycle_threshold: u8,
    /// BPM の指数平均で最新観測値に与える重み（Q0.8, 1..=255）。
    pub bpm_ema_alpha_q8: u8,
}

impl PulseSyncParam {
    /// 既定値（4周期タイムアウト、EMA重み 96/256）。
    pub const DEFAULT: Self = Self {
        missing_cycle_threshold: 4,
        bpm_ema_alpha_q8: 96,
    };

    #[inline]
    pub const fn new(missing_cycle_threshold: u8, bpm_ema_alpha_q8: u8) -> Self {
        Self {
            missing_cycle_threshold,
            bpm_ema_alpha_q8,
        }
    }

    /// `missing_cycle_threshold >= 1` かつ `1 <= bpm_ema_alpha_q8 <= 255` に正規化する。
    #[inline]
    pub const fn sanitize(self) -> Self {
        let missing_cycle_threshold = if self.missing_cycle_threshold == 0 {
            1
        } else {
            self.missing_cycle_threshold
        };
        let bpm_ema_alpha_q8 = if self.bpm_ema_alpha_q8 == 0 {
            1
        } else {
            self.bpm_ema_alpha_q8
        };
        Self {
            missing_cycle_threshold,
            bpm_ema_alpha_q8,
        }
    }
}

impl Default for PulseSyncParam {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 既存サンプル互換のためのエイリアス。
pub type Rhythm = RhythmGenerator;

/// ネットワーク共有用メッセージ
///
/// 16バイトの固定レイアウトで、UDPマルチキャストなどで送受信することを想定する。
/// シリアライズはリトルエンディアンで行い、受信側で環境に応じて変換する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[repr(C)]
pub struct RhythmMessage {
    pub timestamp_ms: u64,
    pub beat_count: u32,
    pub phase: u16,
    pub bpm: BpmQ8,
}

impl RhythmMessage {
    /// 送信タイムスタンプ (u64) のオフセット。
    pub const WIRE_TIMESTAMP_OFFSET: usize = 0;
    /// beat_count (u32) のオフセット。
    pub const WIRE_BEAT_COUNT_OFFSET: usize = 8;
    /// phase (u16) のオフセット。
    pub const WIRE_PHASE_OFFSET: usize = 12;
    /// bpm (u16) のオフセット。
    pub const WIRE_BPM_OFFSET: usize = 14;

    pub const fn new(timestamp_ms: u64, beat_count: u32, phase: u16, bpm: BpmQ8) -> Self {
        Self {
            timestamp_ms,
            beat_count,
            phase,
            bpm,
        }
    }

    /// BigEndian環境の場合は Little Endian に変換してからシリアライズする。
    pub fn to_wire_bytes(self) -> [u8; RHYTHM_MESSAGE_WIRE_SIZE] {
        let mut buf = [0u8; RHYTHM_MESSAGE_WIRE_SIZE];
        buf[Self::WIRE_TIMESTAMP_OFFSET..Self::WIRE_TIMESTAMP_OFFSET + 8]
            .copy_from_slice(&self.timestamp_ms.to_le_bytes());
        buf[Self::WIRE_BEAT_COUNT_OFFSET..Self::WIRE_BEAT_COUNT_OFFSET + 4]
            .copy_from_slice(&self.beat_count.to_le_bytes());
        buf[Self::WIRE_PHASE_OFFSET..Self::WIRE_PHASE_OFFSET + 2]
            .copy_from_slice(&self.phase.to_le_bytes());
        buf[Self::WIRE_BPM_OFFSET..Self::WIRE_BPM_OFFSET + 2]
            .copy_from_slice(&self.bpm.to_int_round().to_le_bytes());
        buf
    }

    /// リトルエンディアンのバイトスライスからデシリアライズする。
    pub fn from_wire_slice(buf: &[u8]) -> Option<Self> {
        if buf.len() < RHYTHM_MESSAGE_WIRE_SIZE {
            return None;
        }
        let timestamp_ms = u64::from_le_bytes(
            buf[Self::WIRE_TIMESTAMP_OFFSET..Self::WIRE_TIMESTAMP_OFFSET + 8]
                .try_into()
                .ok()?,
        );
        let beat_count = u32::from_le_bytes(
            buf[Self::WIRE_BEAT_COUNT_OFFSET..Self::WIRE_BEAT_COUNT_OFFSET + 4]
                .try_into()
                .ok()?,
        );
        let phase = u16::from_le_bytes(
            buf[Self::WIRE_PHASE_OFFSET..Self::WIRE_PHASE_OFFSET + 2]
                .try_into()
                .ok()?,
        );
        let bpm = u16::from_le_bytes(
            buf[Self::WIRE_BPM_OFFSET..Self::WIRE_BPM_OFFSET + 2]
                .try_into()
                .ok()?,
        );
        Some(Self {
            timestamp_ms,
            beat_count,
            phase,
            bpm: BpmQ8::from_int(bpm),
        })
    }
}

/// 位相生成と外部同期（蔵本モデル）を担当するコア。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RhythmGenerator {
    pub phase: u16,
    pub base_bpm: BpmQ8,
    pub current_bpm: BpmQ8,
    pub beat_count: u64,
    pub sync_state: SyncState,

    coupling_divisor: u16,
    phase_accum_sub16: u64,
    first_point_ts_ms: Option<u64>,
    first_point_beat_count: Option<u32>,
    last_sync_ts_ms: Option<u64>,
    last_sync_beat_count: Option<u32>,
    pulse_last_ts_ms: Option<u64>,
    pulse_silence_ms: u64,
    pulse_sync_param: PulseSyncParam,
    pulse_interval_ms_ring: [u32; PULSE_INTERVAL_HISTORY_SIZE],
    pulse_interval_len: u8,
    pulse_interval_next_idx: u8,
}

/// 低頻度入力でのジッタ耐性を持たせるための同期状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncState {
    /// 自律動作。
    Idle,
    /// 1点目受信後。2点目待ち。
    WaitSecondPoint,
    /// 2点観測済み。BPM確定済み。
    Locked,
}

impl RhythmGenerator {
    /// Q8.8 BPM 指定でジェネレータを生成する。
    pub fn new(phase: u16, bpm: BpmQ8, coupling_divisor: u16) -> Self {
        Self {
            phase,
            base_bpm: bpm,
            current_bpm: bpm,
            beat_count: 0,
            sync_state: SyncState::Idle,
            coupling_divisor: coupling_divisor.max(2),
            phase_accum_sub16: (phase as u64) << 16,
            first_point_ts_ms: None,
            first_point_beat_count: None,
            last_sync_ts_ms: None,
            last_sync_beat_count: None,
            pulse_last_ts_ms: None,
            pulse_silence_ms: 0,
            pulse_sync_param: PulseSyncParam::default(),
            pulse_interval_ms_ring: [0; PULSE_INTERVAL_HISTORY_SIZE],
            pulse_interval_len: 0,
            pulse_interval_next_idx: 0,
        }
    }

    /// 整数 BPM 指定でジェネレータを生成する。
    pub fn from_int_bpm(phase: u16, bpm: u16, coupling_divisor: u16) -> Self {
        Self::new(phase, BpmQ8::from_int(bpm), coupling_divisor)
    }

    /// `sync_pulse` の同期パラメータを更新する。
    pub fn set_pulse_sync_param(&mut self, pulse_sync_param: PulseSyncParam) {
        self.pulse_sync_param = pulse_sync_param.sanitize();
    }

    /// 現在の `sync_pulse` 同期パラメータを返す。
    #[inline]
    pub fn pulse_sync_param(&self) -> PulseSyncParam {
        self.pulse_sync_param
    }

    /// 自律更新。パケット不在時は `base_bpm` を目標に `current_bpm` を徐々に戻す。
    /// BPM 値は毎回 `bpm_limit_param` の範囲に制約される。
    pub fn update(&mut self, dt_ms: u32, bpm_limit_param: &BpmLimitParam) {
        if dt_ms == 0 {
            return;
        }

        let bpm_limit_param = bpm_limit_param.sanitize();
        self.base_bpm = self.base_bpm.clamp_with_limit(&bpm_limit_param);
        self.current_bpm = self.current_bpm.clamp_with_limit(&bpm_limit_param);

        self.current_bpm = self.current_bpm.blend_toward_with_limit(
            self.base_bpm,
            self.coupling_divisor as i32,
            &bpm_limit_param,
        );

        let before_cycles = self.phase_accum_sub16 >> 32;
        let delta_sub16 = phase_advance_sub16(self.current_bpm.raw(), dt_ms);
        self.phase_accum_sub16 = self.phase_accum_sub16.wrapping_add(delta_sub16);
        let after_cycles = self.phase_accum_sub16 >> 32;

        let wraps = after_cycles.saturating_sub(before_cycles);
        self.beat_count = self.beat_count.saturating_add(wraps);
        self.phase = ((self.phase_accum_sub16 >> 16) & u16::MAX as u64) as u16;

        self.handle_pulse_timeout(dt_ms, &bpm_limit_param);
    }

    /// 外部メッセージに同期する。
    ///
    /// - 遅延補償: `now_ms - msg.timestamp_ms` ぶん相手位相を進める
    /// - 2点観測: 1点目で待機、2点目で `timestamp_ms` と `beat_count` の差分から BPM 確定
    /// - BPM 範囲外: 2:1 分周で 60-120 BPM に折りたたむ
    /// - 位相: 90度刻みオフセットを候補に、現在位相へ最短の点へ吸着
    pub fn sync(&mut self, msg: RhythmMessage, now_ms: u64, bpm_limit_param: &BpmLimitParam) {
        let bpm_limit_param = bpm_limit_param.sanitize();
        self.reset_pulse_tracking();

        let compensated_remote_phase = compensated_remote_phase(msg, now_ms);
        let phase_target = if is_bpm_in_primary_range(msg.bpm, &bpm_limit_param) {
            compensated_remote_phase
        } else {
            nearest_quarter_phase(self.phase, compensated_remote_phase)
        };
        let hinted_bpm = normalize_bpm_to_primary_range(msg.bpm, &bpm_limit_param);

        match self.sync_state {
            SyncState::Idle => {
                self.sync_state = SyncState::WaitSecondPoint;
                self.first_point_ts_ms = Some(msg.timestamp_ms);
                self.first_point_beat_count = Some(msg.beat_count);

                // 1点目でもリズム感を外さないよう、ヒントBPMへ強めに寄せる。
                self.base_bpm = hinted_bpm;
                self.current_bpm =
                    self.current_bpm
                        .blend_toward_with_limit(hinted_bpm, 2, &bpm_limit_param);
                self.force_phase(phase_target);
            }
            SyncState::WaitSecondPoint => {
                let maybe_observed_bpm = self
                    .first_point_ts_ms
                    .zip(self.first_point_beat_count)
                    .and_then(|(first_ts, first_beat_count)| {
                        observed_bpm_from_points(
                            first_ts,
                            first_beat_count,
                            msg.timestamp_ms,
                            msg.beat_count,
                        )
                    });

                if let Some(observed_raw_bpm) = maybe_observed_bpm {
                    let observed_bpm =
                        normalize_bpm_to_primary_range(observed_raw_bpm, &bpm_limit_param);
                    self.base_bpm = observed_bpm;
                    self.current_bpm = observed_bpm;
                    self.sync_state = SyncState::Locked;
                    self.first_point_ts_ms = None;
                    self.first_point_beat_count = None;
                } else {
                    self.first_point_ts_ms = Some(msg.timestamp_ms);
                    self.first_point_beat_count = Some(msg.beat_count);
                    self.base_bpm = hinted_bpm;
                    self.current_bpm =
                        self.current_bpm
                            .blend_toward_with_limit(hinted_bpm, 2, &bpm_limit_param);
                }
                self.force_phase(phase_target);
            }
            SyncState::Locked => {
                if let Some(observed_raw_bpm) = self
                    .last_sync_ts_ms
                    .zip(self.last_sync_beat_count)
                    .and_then(|(last_ts_ms, last_beat_count)| {
                        observed_bpm_from_points(
                            last_ts_ms,
                            last_beat_count,
                            msg.timestamp_ms,
                            msg.beat_count,
                        )
                    })
                {
                    let observed_bpm =
                        normalize_bpm_to_primary_range(observed_raw_bpm, &bpm_limit_param);
                    // ロック中はジッタを抑えつつ追従する。
                    self.base_bpm =
                        self.base_bpm
                            .blend_toward_with_limit(observed_bpm, 4, &bpm_limit_param);
                }

                self.base_bpm =
                    self.base_bpm
                        .blend_toward_with_limit(hinted_bpm, 4, &bpm_limit_param);
                self.current_bpm =
                    self.current_bpm
                        .blend_toward_with_limit(self.base_bpm, 2, &bpm_limit_param);
                self.force_phase(phase_target);
            }
        }

        self.last_sync_ts_ms = Some(msg.timestamp_ms);
        self.last_sync_beat_count = Some(msg.beat_count);
    }

    /// パルス時刻のみ（BPMヒントなし）で同期する。
    ///
    /// - パルス間隔の指数平均（EMA）から BPM を推定する
    /// - 推定 BPM を `bpm_limit_param` で制約し、範囲外なら 2:1 分周で折りたたむ
    /// - 想定拍でパルス欠落が一定周期続いた場合は `WaitSecondPoint` へ戻す
    /// - 受信遅延分を補償して位相を補正する
    pub fn sync_pulse(&mut self, pulse_ts_ms: u64, now_ms: u64, bpm_limit_param: &BpmLimitParam) {
        let bpm_limit_param = bpm_limit_param.sanitize();
        let pulse_sync_param = self.pulse_sync_param.sanitize();

        let mut maybe_interval_ms = self
            .pulse_last_ts_ms
            .filter(|last_ts| pulse_ts_ms > *last_ts)
            .map(|last_ts| (pulse_ts_ms - last_ts).min(u32::MAX as u64) as u32)
            .filter(|interval_ms| *interval_ms > 0);

        if let Some(interval_ms) = maybe_interval_ms {
            if self.sync_state == SyncState::Locked
                && self.is_pulse_interval_timed_out(
                    interval_ms as u64,
                    &bpm_limit_param,
                    &pulse_sync_param,
                )
            {
                // 長時間ヒントが途切れたパルスは新規キャリブレーションとして扱う。
                self.transition_to_wait_second_point();
                maybe_interval_ms = None;
            }
        }

        if let Some(interval_ms) = maybe_interval_ms {
            self.push_pulse_interval(interval_ms);
        }

        let raw_latest_bpm = maybe_interval_ms.map(BpmQ8::from_interval_ms);
        let raw_smoothed_bpm = self
            .estimated_pulse_bpm(&pulse_sync_param)
            .or(raw_latest_bpm);

        let raw_pulse_bpm = raw_latest_bpm
            .or(raw_smoothed_bpm)
            .unwrap_or(self.current_bpm);
        let pulse_bpm = normalize_bpm_to_primary_range(raw_pulse_bpm, &bpm_limit_param);
        let smoothed_pulse_bpm = normalize_bpm_to_primary_range(
            raw_smoothed_bpm.unwrap_or(raw_pulse_bpm),
            &bpm_limit_param,
        );

        let compensated_pulse_phase = compensated_pulse_phase(pulse_ts_ms, now_ms, pulse_bpm);
        let phase_target = if is_bpm_in_primary_range(raw_pulse_bpm, &bpm_limit_param) {
            compensated_pulse_phase
        } else {
            nearest_quarter_phase(self.phase, compensated_pulse_phase)
        };

        match self.sync_state {
            SyncState::Idle => {
                self.sync_state = SyncState::WaitSecondPoint;
                self.first_point_ts_ms = Some(pulse_ts_ms);
                self.first_point_beat_count = None;
                self.base_bpm = pulse_bpm;
                self.current_bpm =
                    self.current_bpm
                        .blend_toward_with_limit(pulse_bpm, 2, &bpm_limit_param);
                self.force_phase(phase_target);
            }
            SyncState::WaitSecondPoint => {
                if maybe_interval_ms.is_some() {
                    self.base_bpm = smoothed_pulse_bpm;
                    self.current_bpm = smoothed_pulse_bpm;
                    self.sync_state = SyncState::Locked;
                    self.first_point_ts_ms = None;
                    self.first_point_beat_count = None;
                } else {
                    self.first_point_ts_ms = Some(pulse_ts_ms);
                    self.first_point_beat_count = None;
                    self.base_bpm = pulse_bpm;
                    self.current_bpm =
                        self.current_bpm
                            .blend_toward_with_limit(pulse_bpm, 2, &bpm_limit_param);
                }
                self.force_phase(phase_target);
            }
            SyncState::Locked => {
                if maybe_interval_ms.is_some() {
                    self.base_bpm = smoothed_pulse_bpm;

                    self.current_bpm = self.current_bpm.blend_toward_with_limit(
                        smoothed_pulse_bpm,
                        2,
                        &bpm_limit_param,
                    );
                } else {
                    self.current_bpm = self.current_bpm.blend_toward_with_limit(
                        self.base_bpm,
                        2,
                        &bpm_limit_param,
                    );
                }
                self.force_phase(phase_target);
            }
        }

        self.pulse_last_ts_ms = Some(pulse_ts_ms);
        self.pulse_silence_ms = 0;
        self.last_sync_ts_ms = Some(pulse_ts_ms);
        self.last_sync_beat_count = None;
    }

    pub fn to_message(&self, now_ms: u64) -> RhythmMessage {
        RhythmMessage {
            timestamp_ms: now_ms,
            beat_count: self.beat_count.min(u32::MAX as u64) as u32,
            phase: self.phase,
            bpm: self.current_bpm,
        }
    }

    #[inline]
    fn force_phase(&mut self, phase: u16) {
        self.phase = phase;
        let cycles = self.phase_accum_sub16 >> 32;
        self.phase_accum_sub16 = (cycles << 32) | ((phase as u64) << 16);
    }

    #[inline]
    fn handle_pulse_timeout(&mut self, dt_ms: u32, bpm_limit_param: &BpmLimitParam) {
        if self.sync_state != SyncState::Locked || self.pulse_last_ts_ms.is_none() {
            return;
        }

        self.pulse_silence_ms = self.pulse_silence_ms.saturating_add(dt_ms as u64);
        let pulse_sync_param = self.pulse_sync_param.sanitize();
        if self.is_pulse_interval_timed_out(
            self.pulse_silence_ms,
            bpm_limit_param,
            &pulse_sync_param,
        ) {
            self.transition_to_wait_second_point();
        }
    }

    #[inline]
    fn is_pulse_interval_timed_out(
        &self,
        interval_ms: u64,
        bpm_limit_param: &BpmLimitParam,
        pulse_sync_param: &PulseSyncParam,
    ) -> bool {
        let expected_interval_ms =
            expected_interval_ms_from_bpm(self.base_bpm.clamp_with_limit(bpm_limit_param));
        let timeout_ms =
            expected_interval_ms.saturating_mul(pulse_sync_param.missing_cycle_threshold as u64);
        timeout_ms > 0 && interval_ms >= timeout_ms
    }

    #[inline]
    fn transition_to_wait_second_point(&mut self) {
        self.sync_state = SyncState::WaitSecondPoint;
        self.first_point_ts_ms = None;
        self.first_point_beat_count = None;
        self.reset_pulse_tracking();
    }

    #[inline]
    fn reset_pulse_tracking(&mut self) {
        self.pulse_last_ts_ms = None;
        self.pulse_silence_ms = 0;
        self.pulse_interval_ms_ring = [0; PULSE_INTERVAL_HISTORY_SIZE];
        self.pulse_interval_len = 0;
        self.pulse_interval_next_idx = 0;
    }

    #[inline]
    fn push_pulse_interval(&mut self, interval_ms: u32) {
        if interval_ms == 0 {
            return;
        }

        let idx = self.pulse_interval_next_idx as usize % PULSE_INTERVAL_HISTORY_SIZE;
        self.pulse_interval_ms_ring[idx] = interval_ms;
        self.pulse_interval_next_idx = ((idx + 1) % PULSE_INTERVAL_HISTORY_SIZE) as u8;
        if self.pulse_interval_len < PULSE_INTERVAL_HISTORY_SIZE as u8 {
            self.pulse_interval_len += 1;
        }
    }

    #[inline]
    fn estimated_pulse_bpm(&self, pulse_sync_param: &PulseSyncParam) -> Option<BpmQ8> {
        let len = self.pulse_interval_len as usize;
        if len == 0 {
            return None;
        }

        let pulse_sync_param = pulse_sync_param.sanitize();
        let alpha = pulse_sync_param.bpm_ema_alpha_q8 as u32;
        let beta = EMA_Q8_ONE.saturating_sub(alpha);
        let ring_size = PULSE_INTERVAL_HISTORY_SIZE;
        let start_idx = if len < ring_size {
            0
        } else {
            self.pulse_interval_next_idx as usize % ring_size
        };

        let mut ema_raw_q8: Option<u32> = None;
        for offset in 0..len {
            let idx = (start_idx + offset) % ring_size;
            let interval_ms = self.pulse_interval_ms_ring[idx];
            if interval_ms == 0 {
                continue;
            }

            let bpm_raw_q8 = BpmQ8::from_interval_ms(interval_ms).raw() as u32;
            ema_raw_q8 = Some(match ema_raw_q8 {
                Some(prev_raw_q8) => {
                    (prev_raw_q8.saturating_mul(beta)
                        + bpm_raw_q8.saturating_mul(alpha)
                        + EMA_Q8_ONE / 2)
                        / EMA_Q8_ONE
                }
                None => bpm_raw_q8,
            });
        }

        ema_raw_q8.map(|raw| BpmQ8(raw.min(u16::MAX as u32) as u16))
    }
}

#[inline]
fn expected_interval_ms_from_bpm(bpm: BpmQ8) -> u64 {
    let raw_q8 = bpm.raw().max(1) as u64;
    ((MS_PER_MINUTE as u64 * BPM_Q8_ONE as u64) + raw_q8 / 2) / raw_q8
}

/// 観測2点（時刻・拍数）から Q8.8 BPM を推定する。
///
/// `beat_count` は `u32` ラップ算術で差分を計算する。
#[inline]
fn observed_bpm_from_points(
    prev_ts_ms: u64,
    prev_beat_count: u32,
    current_ts_ms: u64,
    current_beat_count: u32,
) -> Option<BpmQ8> {
    if current_ts_ms <= prev_ts_ms {
        return None;
    }

    let beat_delta = current_beat_count.wrapping_sub(prev_beat_count);
    if beat_delta == 0 {
        return None;
    }

    let interval_ms = current_ts_ms - prev_ts_ms;
    let numerator = MS_PER_MINUTE as u128 * BPM_Q8_ONE as u128 * beat_delta as u128;
    let raw = ((numerator + interval_ms as u128 / 2) / interval_ms as u128)
        .clamp(BPM_Q8_ONE as u128, u16::MAX as u128) as u16;
    Some(BpmQ8(raw))
}

#[inline]
fn is_bpm_in_primary_range(bpm: BpmQ8, bpm_limit_param: &BpmLimitParam) -> bool {
    bpm_limit_param.contains_q8(bpm)
}

#[inline]
fn compensated_remote_phase(msg: RhythmMessage, now_ms: u64) -> u16 {
    let delay_ms = now_ms.saturating_sub(msg.timestamp_ms).min(u32::MAX as u64) as u32;
    PhaseU16(msg.phase)
        .wrapping_add(PhaseU16::advance(msg.bpm, delay_ms))
        .raw()
}

#[inline]
fn compensated_pulse_phase(pulse_ts_ms: u64, now_ms: u64, bpm: BpmQ8) -> u16 {
    let delay_ms = now_ms.saturating_sub(pulse_ts_ms).min(u32::MAX as u64) as u32;
    PhaseU16::advance(bpm, delay_ms).raw()
}

#[inline]
fn nearest_quarter_phase(current_phase: u16, remote_phase: u16) -> u16 {
    const QUARTER_OFFSETS: [u16; 4] = [0, 16_384, 32_768, 49_152];

    let mut best = remote_phase;
    let mut best_abs = i32::MAX;
    for offset in QUARTER_OFFSETS {
        let candidate = remote_phase.wrapping_add(offset);
        let abs = (PhaseU16(candidate).signed_diff(PhaseU16(current_phase)) as i32).abs();
        if abs < best_abs {
            best_abs = abs;
            best = candidate;
        }
    }
    best
}

#[inline]
fn normalize_bpm_to_primary_range(bpm: BpmQ8, bpm_limit_param: &BpmLimitParam) -> BpmQ8 {
    let bpm_limit_param = bpm_limit_param.sanitize();
    let min = bpm_limit_param.min_q8().raw() as u32;
    let max = bpm_limit_param.max_q8().raw() as u32;

    let mut raw = bpm.raw() as u32;
    if raw == 0 {
        return bpm_limit_param.min_q8();
    }

    while raw > max {
        // 高すぎるテンポは 2:1 分周へ折りたたむ。
        raw = raw.div_ceil(2);
    }
    while raw < min {
        raw = raw.saturating_mul(2);
        if raw == 0 {
            return bpm_limit_param.min_q8();
        }
    }

    BpmQ8(raw as u16).clamp_with_limit(&bpm_limit_param)
}

#[cfg(test)]
mod tests {
    use super::*;
    use consts::RHYTHM_MESSAGE_WIRE_SIZE;

    // ── 正常系: シリアライズ→デシリアライズの往復 ─────────────────────────────────

    /// to_wire_bytes → from_wire_slice の往復で元の値に戻ること。
    #[test]
    fn wire_bytes_roundtrip() {
        let cases: &[(&str, RhythmMessage)] = &[
            ("全ゼロ（デフォルト）", RhythmMessage::default()),
            (
                "120 BPM / 標準",
                RhythmMessage::new(1_000, 1, 0, BpmQ8::from_int(120)),
            ),
            (
                "90 BPM / 標準",
                RhythmMessage::new(2_000, 10, 32768, BpmQ8::from_int(90)),
            ),
            (
                "60 BPM / 標準",
                RhythmMessage::new(3_000, 100, 65535, BpmQ8::from_int(60)),
            ),
            (
                "タイムスタンプ最大値",
                RhythmMessage::new(u64::MAX, 0, 0, BpmQ8::from_int(120)),
            ),
            (
                "beat_count 最大値",
                RhythmMessage::new(0, u32::MAX, 0, BpmQ8::from_int(90)),
            ),
            (
                "phase 最大値",
                RhythmMessage::new(0, 0, u16::MAX, BpmQ8::from_int(60)),
            ),
            (
                "全フィールド境界値",
                RhythmMessage::new(u64::MAX, u32::MAX, u16::MAX, BpmQ8::from_int(120)),
            ),
        ];
        for (label, msg) in cases {
            let buf = msg.to_wire_bytes();
            let restored = RhythmMessage::from_wire_slice(&buf)
                .unwrap_or_else(|| panic!("[{label}] from_wire_slice が None を返した"));
            assert_eq!(restored, *msg, "[{label}] 往復後の値が一致しない");
        }
    }

    // ── 値域確認: ワイヤーバイトの各フィールドのバイト位置 ─────────────────────────

    /// 各フィールドが仕様のオフセット位置に正しく書き込まれていること。
    #[test]
    fn wire_bytes_field_layout() {
        // (label, msg, timestamp_ms, beat_count, phase, bpm_int)
        let cases: &[(&str, RhythmMessage, u64, u32, u16, u16)] = &[
            (
                "通常値",
                RhythmMessage::new(
                    0x0102_0304_0506_0708,
                    0xDEAD_BEEF,
                    0xABCD,
                    BpmQ8::from_int(120),
                ),
                0x0102_0304_0506_0708,
                0xDEAD_BEEF,
                0xABCD,
                120,
            ),
            ("全ゼロ", RhythmMessage::default(), 0, 0, 0, 0),
        ];
        for (label, msg, ts, bc, ph, bpm_int) in cases {
            let buf = msg.to_wire_bytes();
            assert_eq!(
                u64::from_le_bytes(
                    buf[RhythmMessage::WIRE_TIMESTAMP_OFFSET..][..8]
                        .try_into()
                        .unwrap()
                ),
                *ts,
                "[{label}] timestamp_ms",
            );
            assert_eq!(
                u32::from_le_bytes(
                    buf[RhythmMessage::WIRE_BEAT_COUNT_OFFSET..][..4]
                        .try_into()
                        .unwrap()
                ),
                *bc,
                "[{label}] beat_count",
            );
            assert_eq!(
                u16::from_le_bytes(
                    buf[RhythmMessage::WIRE_PHASE_OFFSET..][..2]
                        .try_into()
                        .unwrap()
                ),
                *ph,
                "[{label}] phase",
            );
            assert_eq!(
                u16::from_le_bytes(
                    buf[RhythmMessage::WIRE_BPM_OFFSET..][..2]
                        .try_into()
                        .unwrap()
                ),
                *bpm_int,
                "[{label}] bpm",
            );
        }
    }

    // ── 異常系: バッファ不足で None を返す ─────────────────────────────────────────

    /// バッファ長が WIRE_SIZE 未満の場合は from_wire_slice が None を返すこと。
    #[test]
    fn wire_bytes_short_buffer_returns_none() {
        for len in 0..RHYTHM_MESSAGE_WIRE_SIZE {
            let short = vec![0u8; len];
            assert!(
                RhythmMessage::from_wire_slice(&short).is_none(),
                "長さ {len} のバッファで None が返らなかった"
            );
        }
    }
}
