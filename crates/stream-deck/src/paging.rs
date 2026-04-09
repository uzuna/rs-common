//! ページング共通操作モジュール。
//!
//! SSH・Podman など複数ページで構成されるページグループ共通の UI 操作を定義する。
//! - `PageNavOverlay`: ページ遷移操作直後に左端 LCD セクションを一時的にページ位置情報で上書きする状態管理

use std::time::{Duration, Instant};

use crate::renderer::SectionSpec;

/// ページ遷移直後オーバーレイの表示継続時間。
pub const OVERLAY_DURATION: Duration = Duration::from_secs(2);

/// ページ遷移操作後に左端 LCD セクションを一時上書きする状態管理。
///
/// `trigger()` を呼ぶことで `OVERLAY_DURATION` の間だけ `is_active()` が `true` になる。
/// エンコーダ操作・ボタン操作どちらのページ遷移でも同じインスタンスを使い、
/// `build_section_spec()` でオーバーレイ用 `SectionSpec` を取得して左端セクションに上書きする。
pub struct PageNavOverlay {
    activated_at: Option<Instant>,
}

impl PageNavOverlay {
    pub fn new() -> Self {
        Self { activated_at: None }
    }

    /// ページ遷移操作が発生したことを記録する。
    pub fn trigger(&mut self) {
        self.activated_at = Some(Instant::now());
    }

    /// `now` 時点でオーバーレイが有効かどうかを返す。
    pub fn is_active(&self, now: Instant) -> bool {
        self.activated_at
            .map(|t| now.saturating_duration_since(t) < OVERLAY_DURATION)
            .unwrap_or(false)
    }

    /// ページタイトルからオーバーレイ用 `SectionSpec` を生成する。
    ///
    /// タイトルは動的ページ生成時に `"Podman 2 / 4"` 形式で付与される。
    /// `" / "` を含まない場合や数値のパースに失敗した場合は `None` を返す。
    ///
    /// # 引数
    /// - `page_title`: 現在ページのタイトル（例: `"Podman 2 / 4"`, `"SSH 1 / 3"`）
    /// - `history_len`: 進捗バーのバー本数（呼び出し元の `HISTORY_LEN` を渡す）
    pub fn build_section_spec(page_title: &str, history_len: usize) -> Option<SectionSpec> {
        // "Podman 2 / 4" → split by " / " → ["Podman 2", "4"]
        let (left, total_str) = page_title.split_once(" / ")?;
        let total: usize = total_str.trim().parse().ok()?;

        // "Podman 2" → last token is current page index
        let current_str = left.split_whitespace().last()?;
        let current: usize = current_str.parse().ok()?;

        if total == 0 || current == 0 {
            return None;
        }

        // label: 数字より前の単語部分。"Podman 2" → "Podman"
        let label = left
            .split_whitespace()
            .rev()
            .skip(1) // skip current number
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join(" ");
        let label = if label.is_empty() {
            "Page".to_string()
        } else {
            label
        };

        // 進捗バー: current が 1 なら 0.0、total なら 1.0
        let progress = if total <= 1 {
            1.0f32
        } else {
            (current - 1) as f32 / (total - 1) as f32
        };
        let history = vec![progress; history_len.max(1)];

        Some(SectionSpec {
            label,
            value_text: format!("{current} / {total}"),
            history,
        })
    }
}

// ── テスト ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_HISTORY_LEN: usize = 10;

    /// 値域確認: " / " を含まないタイトルは None
    #[test]
    fn test_build_section_spec_no_separator() {
        assert!(PageNavOverlay::build_section_spec("Home", TEST_HISTORY_LEN).is_none());
        assert!(PageNavOverlay::build_section_spec("", TEST_HISTORY_LEN).is_none());
    }

    /// 値域確認: 数値パース失敗は None
    #[test]
    fn test_build_section_spec_parse_failure() {
        assert!(PageNavOverlay::build_section_spec("Page X / Y", TEST_HISTORY_LEN).is_none());
        assert!(PageNavOverlay::build_section_spec("Podman 1 / Z", TEST_HISTORY_LEN).is_none());
    }

    /// 値域確認: current=0 または total=0 は None
    #[test]
    fn test_build_section_spec_zero_values() {
        assert!(PageNavOverlay::build_section_spec("Podman 0 / 4", TEST_HISTORY_LEN).is_none());
        assert!(PageNavOverlay::build_section_spec("Podman 1 / 0", TEST_HISTORY_LEN).is_none());
    }

    /// 正常系: "Podman 2 / 4" → label="Podman"、value_text="2 / 4"
    #[test]
    fn test_build_section_spec_podman() {
        let spec = PageNavOverlay::build_section_spec("Podman 2 / 4", TEST_HISTORY_LEN).unwrap();
        assert_eq!(spec.label, "Podman");
        assert_eq!(spec.value_text, "2 / 4");
        assert_eq!(spec.history.len(), TEST_HISTORY_LEN);
        // progress = (2-1)/(4-1) = 1/3 ≈ 0.333
        let expected = 1.0f32 / 3.0f32;
        for v in &spec.history {
            assert!((v - expected).abs() < 1e-5, "progress={v}");
        }
    }

    /// 正常系: "SSH 1 / 1" → history=[1.0; len]（全ページが1枚）
    #[test]
    fn test_build_section_spec_single_page() {
        let spec = PageNavOverlay::build_section_spec("SSH 1 / 1", TEST_HISTORY_LEN).unwrap();
        assert_eq!(spec.label, "SSH");
        assert_eq!(spec.value_text, "1 / 1");
        for v in &spec.history {
            assert!((v - 1.0).abs() < 1e-5);
        }
    }

    /// 正常系: "SSH 1 / 3" → progress=0.0（先頭ページ）
    #[test]
    fn test_build_section_spec_first_page() {
        let spec = PageNavOverlay::build_section_spec("SSH 1 / 3", TEST_HISTORY_LEN).unwrap();
        assert_eq!(spec.value_text, "1 / 3");
        for v in &spec.history {
            assert!((v - 0.0).abs() < 1e-5);
        }
    }

    /// 正常系: trigger 直後は is_active=true
    #[test]
    fn test_is_active_after_trigger() {
        let mut overlay = PageNavOverlay::new();
        assert!(!overlay.is_active(Instant::now()));
        overlay.trigger();
        assert!(overlay.is_active(Instant::now()));
    }

    /// 異常系: trigger なしは is_active=false
    #[test]
    fn test_is_active_without_trigger() {
        let overlay = PageNavOverlay::new();
        assert!(!overlay.is_active(Instant::now()));
    }

    /// 異常系: OVERLAY_DURATION 経過後は is_active=false
    #[test]
    fn test_is_active_expired() {
        let mut overlay = PageNavOverlay::new();
        overlay.trigger();
        // OVERLAY_DURATION + 1ms 後をシミュレート
        let past = Instant::now() - OVERLAY_DURATION - Duration::from_millis(1);
        // activated_at を過去に設定してテスト
        overlay.activated_at = Some(past);
        assert!(!overlay.is_active(Instant::now()));
    }
}
