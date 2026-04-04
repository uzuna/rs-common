//! ボタン表示サンプルの描画実装。

use std::sync::OnceLock;

use ab_glyph::{FontVec, PxScale};
use image::{DynamicImage, Rgb, RgbImage};
use imageproc::drawing::{draw_filled_rect_mut, draw_hollow_rect_mut, draw_text_mut};
use imageproc::rect::Rect;

use crate::config::DisplaySeverity;
use crate::display::model::ButtonSampleSpec;

const BUTTON_SIZE: u32 = 120;
const FONT_SIZE_TITLE: f32 = 18.0;
const FONT_SIZE_SUB: f32 = 14.0;
const FONT_SIZE_VALUE: f32 = 32.0;
const TEXT_COLOR: Rgb<u8> = Rgb([255, 255, 255]);
const FONT_DATA: &[u8] = include_bytes!("../../assets/NotoSans-Regular.ttf");

static BUTTON_FONT: OnceLock<FontVec> = OnceLock::new();

fn get_font() -> &'static FontVec {
    BUTTON_FONT
        .get_or_init(|| FontVec::try_from_vec(FONT_DATA.to_vec()).expect("フォントロード失敗"))
}

fn severity_color(severity: DisplaySeverity) -> Rgb<u8> {
    match severity {
        DisplaySeverity::Normal => Rgb([0, 160, 80]),
        DisplaySeverity::Warn => Rgb([220, 160, 0]),
        DisplaySeverity::Error => Rgb([190, 30, 30]),
    }
}

pub fn render_button_pattern(sample: &ButtonSampleSpec) -> anyhow::Result<DynamicImage> {
    let font = get_font();
    let img = match sample {
        ButtonSampleSpec::LabelOnly(payload) => {
            let mut img = RgbImage::from_pixel(BUTTON_SIZE, BUTTON_SIZE, Rgb(payload.bg_color));
            draw_text_mut(
                &mut img,
                TEXT_COLOR,
                10,
                ((BUTTON_SIZE as f32 - FONT_SIZE_TITLE) / 2.0) as i32,
                PxScale::from(FONT_SIZE_TITLE),
                font,
                &payload.label,
            );
            img
        }
        ButtonSampleSpec::LabelValue(payload) => {
            let bg = severity_color(payload.severity);
            let mut img = RgbImage::from_pixel(BUTTON_SIZE, BUTTON_SIZE, bg);
            draw_text_mut(
                &mut img,
                TEXT_COLOR,
                8,
                8,
                PxScale::from(FONT_SIZE_SUB),
                font,
                &payload.title,
            );
            draw_text_mut(
                &mut img,
                TEXT_COLOR,
                8,
                38,
                PxScale::from(FONT_SIZE_VALUE),
                font,
                &payload.value,
            );
            draw_text_mut(
                &mut img,
                TEXT_COLOR,
                86,
                78,
                PxScale::from(FONT_SIZE_SUB),
                font,
                &payload.unit,
            );
            img
        }
        ButtonSampleSpec::IconBadge(payload) => {
            let mut img = RgbImage::from_pixel(BUTTON_SIZE, BUTTON_SIZE, Rgb([24, 24, 48]));
            draw_hollow_rect_mut(
                &mut img,
                Rect::at(6, 6).of_size(BUTTON_SIZE - 12, BUTTON_SIZE - 12),
                Rgb([160, 160, 180]),
            );
            draw_text_mut(
                &mut img,
                TEXT_COLOR,
                10,
                44,
                PxScale::from(30.0),
                font,
                &payload.icon,
            );

            let badge_color = severity_color(payload.status);
            draw_filled_rect_mut(&mut img, Rect::at(78, 6).of_size(36, 24), badge_color);
            draw_text_mut(
                &mut img,
                TEXT_COLOR,
                84,
                10,
                PxScale::from(FONT_SIZE_SUB),
                font,
                &payload.badge_count.to_string(),
            );
            img
        }
        ButtonSampleSpec::BarTrend(payload) => {
            let mut img = RgbImage::from_pixel(BUTTON_SIZE, BUTTON_SIZE, Rgb([20, 20, 20]));
            let max = payload.max.max(1.0);
            let ratio = (payload.current / max).clamp(0.0, 1.0);
            let bar_width = ((BUTTON_SIZE as f32 * ratio).round() as u32).max(1);
            draw_filled_rect_mut(
                &mut img,
                Rect::at(0, 0).of_size(bar_width, 16),
                Rgb([0, 130, 210]),
            );
            draw_text_mut(
                &mut img,
                TEXT_COLOR,
                8,
                20,
                PxScale::from(FONT_SIZE_SUB),
                font,
                &format!("{:.0}/{:.0}", payload.current, payload.max),
            );

            let len = payload.history.len().max(1) as i32;
            for (idx, v) in payload.history.iter().enumerate() {
                let x0 = idx as i32 * BUTTON_SIZE as i32 / len;
                let x1 = ((idx as i32 + 1) * BUTTON_SIZE as i32 / len).max(x0 + 1);
                let normalized = (v / max).clamp(0.0, 1.0);
                let h = (normalized * 50.0).round() as i32;
                let y = 114 - h.max(1);
                draw_filled_rect_mut(
                    &mut img,
                    Rect::at(x0, y).of_size((x1 - x0) as u32, h.max(1) as u32),
                    Rgb([180, 220, 250]),
                );
            }
            img
        }
        ButtonSampleSpec::ErrorFallback(payload) => {
            let mut img = RgbImage::from_pixel(BUTTON_SIZE, BUTTON_SIZE, Rgb([110, 0, 0]));
            draw_text_mut(
                &mut img,
                TEXT_COLOR,
                8,
                12,
                PxScale::from(FONT_SIZE_SUB),
                font,
                &payload.error_code,
            );
            draw_text_mut(
                &mut img,
                TEXT_COLOR,
                8,
                58,
                PxScale::from(FONT_SIZE_TITLE),
                font,
                &payload.message,
            );
            img
        }
    };

    Ok(DynamicImage::ImageRgb8(img))
}
