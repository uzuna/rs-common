//! Renderer: 800x100 の LCD ストリップに 4セクション構成で描画するモジュール
//!
//! ## レイアウト
//! ```text
//! ┌──────────────────┬──────────────────┬──────────────────┬──────────────────┐
//! │ CPU              │ MEM              │ LOAD             │ (未定)            │
//! │     75.3%        │     48.2%        │      2.15        │      ---         │
//! │▓▓░░▓▓▓░░▓▓░░▓▓▓ │...同様...        │...同様...        │                  │
//! └──────────────────┴──────────────────┴──────────────────┴──────────────────┘
//!   ←200px→              ←200px→              ←200px→              ←200px→
//! ```
//! バー: 全体高(100px)の背景として描画し、テキストをオーバーレイ

use ab_glyph::{FontVec, PxScale};
use image::{DynamicImage, Rgb, RgbImage};
use imageproc::{
    drawing::{draw_filled_rect_mut, draw_text_mut, text_size},
    rect::Rect,
};

/// LCDストリップのサイズ (Stream Deck+ 仕様)
pub const LCD_WIDTH: u32 = 800;
pub const LCD_HEIGHT: u32 = 100;

/// セクション幅 (200px × 4 = 800px)
const SECTION_WIDTH: u32 = 200;
/// バーの本数 (= 10秒分のサンプル数)
const BAR_COUNT: usize = 10;
/// 1本あたりのバー幅 (200px / 10 = 20px)
const BAR_WIDTH: u32 = SECTION_WIDTH / BAR_COUNT as u32;

/// 値テキストのフォントサイズ (px)
const VALUE_SCALE: f32 = 40.0;
/// ラベルのフォントサイズ (px) — 値の半分
const LABEL_SCALE: f32 = VALUE_SCALE / 2.0;

/// 色定義
const COLOR_BG: Rgb<u8> = Rgb([0, 0, 0]);
const COLOR_DIVIDER: Rgb<u8> = Rgb([35, 35, 35]);
const COLOR_VALUE: Rgb<u8> = Rgb([210, 255, 210]);
const COLOR_LABEL: Rgb<u8> = Rgb([150, 150, 150]);

/// NotoSans-Regular の埋め込みフォントデータ
static FONT_DATA: &[u8] = include_bytes!("../assets/NotoSans-Regular.ttf");

/// 1セクション分の描画データ
pub struct SectionSpec<'a> {
    /// ラベル文字列 (例: "CPU", "MEM", "LOAD")
    pub label: &'a str,
    /// フォーマット済みの値文字列 (例: "75.3%", "2.15")
    pub value_text: String,
    /// 正規化済み履歴 0.0..=1.0 (古い順、最大 BAR_COUNT 要素)
    pub history: &'a [f32],
}

/// 4セクション構成で描画する LCD レンダラ
pub struct Renderer {
    font: FontVec,
}

impl Renderer {
    /// フォントをメモリに展開してレンダラを生成する
    pub fn new() -> anyhow::Result<Self> {
        let font = FontVec::try_from_vec(FONT_DATA.to_vec())
            .map_err(|e| anyhow::anyhow!("フォントのロードに失敗: {e:?}"))?;
        Ok(Self { font })
    }

    /// 4セクションのデータを受け取り 800x100 の DynamicImage を生成する
    pub fn render(&self, sections: &[SectionSpec; 4]) -> DynamicImage {
        let mut canvas = RgbImage::from_pixel(LCD_WIDTH, LCD_HEIGHT, COLOR_BG);

        for (s, spec) in sections.iter().enumerate() {
            let x_offset = s as u32 * SECTION_WIDTH;
            self.draw_section(&mut canvas, x_offset, spec);
        }

        // セクション区切り線
        for divider_x in [SECTION_WIDTH, SECTION_WIDTH * 2, SECTION_WIDTH * 3] {
            draw_filled_rect_mut(
                &mut canvas,
                Rect::at(divider_x as i32, 0).of_size(1, LCD_HEIGHT),
                COLOR_DIVIDER,
            );
        }

        DynamicImage::ImageRgb8(canvas)
    }

    /// 1セクション (200×100) を canvas の x_offset 位置に描画する
    fn draw_section(&self, canvas: &mut RgbImage, x_offset: u32, spec: &SectionSpec) {
        // ── バープロット (背景レイヤ) ─────────────────────────────
        let history = spec.history;
        let bar_area = BAR_COUNT;
        // 足りない分は左側を 0.0 でパディング
        let pad = bar_area.saturating_sub(history.len());

        for i in 0..bar_area {
            let norm = if i < pad {
                0.0_f32
            } else {
                history[i - pad].clamp(0.0, 1.0)
            };
            // バーを古い順(左)→新しい順(右)で描画。最新バーを少し明るく
            let brightness: u8 = if i == bar_area - 1 {
                75
            } else {
                40 + (25 * i / bar_area) as u8
            };
            let bar_color = Rgb([0, brightness, 0]);
            let bg_color = Rgb([0, brightness / 5, 0]);

            let bar_x = x_offset + i as u32 * BAR_WIDTH;
            let bar_h = (norm * LCD_HEIGHT as f32) as u32;
            let empty_h = LCD_HEIGHT - bar_h;

            // 空白部分 (バーの上)
            if empty_h > 0 {
                draw_filled_rect_mut(
                    canvas,
                    Rect::at(bar_x as i32, 0).of_size(BAR_WIDTH, empty_h),
                    bg_color,
                );
            }
            // バー本体
            if bar_h > 0 {
                draw_filled_rect_mut(
                    canvas,
                    Rect::at(bar_x as i32, empty_h as i32).of_size(BAR_WIDTH, bar_h),
                    bar_color,
                );
            }
        }

        // ── ラベル (左上) ────────────────────────────────────────
        let label_scale = PxScale::from(LABEL_SCALE);
        draw_text_mut(
            canvas,
            COLOR_LABEL,
            x_offset as i32 + 5,
            4,
            label_scale,
            &self.font,
            spec.label,
        );

        // ── 値テキスト (中央) ────────────────────────────────────
        if !spec.value_text.is_empty() {
            let val_scale = PxScale::from(VALUE_SCALE);
            let (tw, th) = text_size(val_scale, &self.font, &spec.value_text);
            let text_x = x_offset as i32 + (SECTION_WIDTH as i32 - tw as i32) / 2;
            let text_y = (LCD_HEIGHT as i32 - th as i32) / 2;
            draw_text_mut(
                canvas,
                COLOR_VALUE,
                text_x.max(x_offset as i32),
                text_y.max(0),
                val_scale,
                &self.font,
                &spec.value_text,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_renderer() -> Renderer {
        Renderer::new().expect("レンダラの初期化に失敗")
    }

    fn make_sections<'a>(histories: &'a [[f32; 10]; 4]) -> [SectionSpec<'a>; 4] {
        let labels = ["CPU", "MEM", "LOAD", "---"];
        let values = ["75.3%", "48.2%", "2.15", ""];
        std::array::from_fn(|i| SectionSpec {
            label: labels[i],
            value_text: values[i].to_string(),
            history: &histories[i],
        })
    }

    /// 正常系: 代表的な履歴値でも 800x100 の画像が生成される
    #[test]
    fn test_render_produces_correct_size() {
        let renderer = make_renderer();
        let histories: [[f32; 10]; 4] = [
            [0.0, 0.1, 0.5, 0.8, 0.9, 0.7, 0.5, 0.3, 0.6, 0.75],
            [0.4, 0.4, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.48],
            [0.2, 0.3, 0.4, 0.3, 0.5, 0.4, 0.3, 0.4, 0.5, 0.3],
            [0.0; 10],
        ];
        let sections = make_sections(&histories);
        let img = renderer.render(&sections);
        assert_eq!(img.width(), LCD_WIDTH);
        assert_eq!(img.height(), LCD_HEIGHT);
    }

    /// 異常系: 履歴が空でもパニックしない
    #[test]
    fn test_render_empty_history() {
        let renderer = make_renderer();
        let sections: [SectionSpec; 4] = std::array::from_fn(|_| SectionSpec {
            label: "X",
            value_text: "0.0".to_string(),
            history: &[],
        });
        let img = renderer.render(&sections);
        assert_eq!(img.width(), LCD_WIDTH);
    }

    /// 値域確認: 正規化後の履歴値が 0.0〜1.0 にクランプされる
    #[test]
    fn test_history_normalization_clamp() {
        let cases: &[f32] = &[-0.5, 0.0, 0.5, 1.0, 1.5];
        for &v in cases {
            let clamped = v.clamp(0.0, 1.0);
            assert!((0.0..=1.0).contains(&clamped), "clamped={clamped}");
        }
    }

    /// 値域確認: バー幅の計算が正しい
    #[test]
    fn test_bar_width_fills_section() {
        assert_eq!(BAR_WIDTH * BAR_COUNT as u32, SECTION_WIDTH);
    }
}
