//! Renderer: 800x100 の LCD ストリップに 4セクション構成で描画するモジュール
//!
//! ## レイアウト
//! セクション数は実行時に決まり、`LCD_WIDTH / n` px 幅で均等分割する。
//! バーの本数は各セクションの `history.len()` に従い、幅も自動計算される。
//!
//! ```text
//! ┌──────────────────┬──────────────────┬──────────────────┬──────────────────┐
//! │ CPU              │ MEM              │ LOAD             │ NOTIF            │
//! │     75.3%        │     48.2%        │      2.15        │      ---         │
//! │▓▓░░▓▓▓░░▓▓░░▓▓▓ │...同様...        │...同様...        │                  │
//! └──────────────────┴──────────────────┴──────────────────┴──────────────────┘
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

/// 1セクション分の描画データ（所有権あり）
pub struct SectionSpec {
    /// ラベル文字列 (例: "CPU", "MEM", "LOAD")
    pub label: &'static str,
    /// フォーマット済みの値文字列 (例: "75.3%", "2.15")
    pub value_text: String,
    /// 正規化済み履歴 0.0..=1.0 (古い順)
    pub history: Vec<f32>,
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

    /// セクション列を受け取り 800x100 の DynamicImage を生成する
    ///
    /// セクション数は可変。幅は `LCD_WIDTH / n` で均等分割し、
    /// 各セクションのバー本数は `spec.history.len()` に従う。
    pub fn render(&self, sections: &[SectionSpec]) -> DynamicImage {
        let n = sections.len().max(1) as u32;
        let section_width = LCD_WIDTH / n;
        let mut canvas = RgbImage::from_pixel(LCD_WIDTH, LCD_HEIGHT, COLOR_BG);

        for (s, spec) in sections.iter().enumerate() {
            let x_offset = s as u32 * section_width;
            self.draw_section(&mut canvas, x_offset, section_width, spec);
        }

        // セクション間の区切り線
        for s in 1..sections.len() {
            let divider_x = s as u32 * section_width;
            draw_filled_rect_mut(
                &mut canvas,
                Rect::at(divider_x as i32, 0).of_size(1, LCD_HEIGHT),
                COLOR_DIVIDER,
            );
        }

        DynamicImage::ImageRgb8(canvas)
    }

    /// 1セクションを canvas の指定位置に描画する
    fn draw_section(
        &self,
        canvas: &mut RgbImage,
        x_offset: u32,
        section_width: u32,
        spec: &SectionSpec,
    ) {
        // ── バープロット (背景レイヤ) ─────────────────────────────
        let bar_count = spec.history.len();
        if bar_count > 0 {
            let bar_width = (section_width / bar_count as u32).max(1);
            for (i, &norm) in spec.history.iter().enumerate() {
                let norm = norm.clamp(0.0, 1.0);
                // バーを古い順(左)→新しい順(右)で描画。最新バーを少し明るく
                let brightness: u8 = if i == bar_count - 1 {
                    75
                } else {
                    40 + (25 * i / bar_count) as u8
                };
                let bar_color = Rgb([0, brightness, 0]);
                let bg_color = Rgb([0, brightness / 5, 0]);

                let bar_x = x_offset + i as u32 * bar_width;
                let bar_h = (norm * LCD_HEIGHT as f32) as u32;
                let empty_h = LCD_HEIGHT - bar_h;

                // 空白部分 (バーの上)
                if empty_h > 0 {
                    draw_filled_rect_mut(
                        canvas,
                        Rect::at(bar_x as i32, 0).of_size(bar_width, empty_h),
                        bg_color,
                    );
                }
                // バー本体
                if bar_h > 0 {
                    draw_filled_rect_mut(
                        canvas,
                        Rect::at(bar_x as i32, empty_h as i32).of_size(bar_width, bar_h),
                        bar_color,
                    );
                }
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
            let text_x = x_offset as i32 + (section_width as i32 - tw as i32) / 2;
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

    fn make_sections(histories: &[[f32; 10]; 4]) -> [SectionSpec; 4] {
        let labels = ["CPU", "MEM", "LOAD", "---"];
        let values = ["75.3%", "48.2%", "2.15", ""];
        std::array::from_fn(|i| SectionSpec {
            label: labels[i],
            value_text: values[i].to_string(),
            history: histories[i].to_vec(),
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
            history: vec![],
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

    /// 正常系: セクション数を変えても LCD 幅を満たす画像が生成される
    #[test]
    fn test_render_variable_section_count() {
        let renderer = make_renderer();
        for n in [1usize, 2, 3, 4, 5, 6] {
            let sections: Vec<SectionSpec> = (0..n)
                .map(|_| SectionSpec {
                    label: "X",
                    value_text: "1.0".to_string(),
                    history: vec![0.5; 10],
                })
                .collect();
            let img = renderer.render(&sections);
            assert_eq!(img.width(), LCD_WIDTH, "n={n} のとき幅が不正");
            assert_eq!(img.height(), LCD_HEIGHT, "n={n} のとき高さが不正");
        }
    }
}
