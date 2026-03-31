//! HardwareManager: Stream Deck+ デバイスへのHIDアクセスを管理するモジュール

use std::sync::Arc;

use anyhow::{Context, Result};
use elgato_streamdeck::{
    images::convert_image_with_format, list_devices, new_hidapi, DeviceStateReader, StreamDeck,
};
use image::{DynamicImage, Rgb, RgbImage};

use crate::error::StreamDeckError;

/// デバイスへのHID接続と画像転送を管理する
pub struct HardwareManager {
    deck: Arc<StreamDeck>,
}

impl HardwareManager {
    /// Stream Deck+ を列挙して最初に見つかったデバイスに接続する
    pub fn connect() -> Result<Self, StreamDeckError> {
        let hid = new_hidapi().context("HidApi の初期化に失敗しました")?;

        let devices = list_devices(&hid);
        let (kind, serial) = devices
            .into_iter()
            .find(|(k, _)| *k == elgato_streamdeck::info::Kind::Plus)
            .ok_or_else(|| {
                StreamDeckError::DeviceNotFound(
                    "Stream Deck+ (PID=0x0084) が見つかりません".to_string(),
                )
            })?;

        tracing::info!(serial = %serial, "Stream Deck+ に接続しました");

        let deck = Arc::new(
            StreamDeck::connect(&hid, kind, &serial)
                .context("Stream Deck+ への接続に失敗しました")?,
        );

        Ok(Self { deck })
    }

    /// LCDストリップ (800x100) に画像を送信する
    ///
    /// `image` は 800x100 の RgbImage を DynamicImage にラップしたもの。
    pub fn set_lcd_strip_image(&self, image: DynamicImage) -> Result<(), StreamDeckError> {
        let format = self.deck.kind().lcd_image_format().ok_or_else(|| {
            StreamDeckError::UnsupportedDevice("LCD フォーマット未定義".to_string())
        })?;

        let image_data =
            convert_image_with_format(format, image).context("JPEG エンコードに失敗しました")?;

        self.deck
            .write_lcd_fill(&image_data)
            .context("LCD への書き込みに失敗しました")?;

        Ok(())
    }

    /// LCD バックライト輝度を設定する (0〜100%)
    pub fn set_brightness(&self, percent: u8) -> Result<(), StreamDeckError> {
        self.deck
            .set_brightness(percent)
            .context("輝度設定に失敗しました")?;
        Ok(())
    }

    /// 入力イベントリーダーを取得する
    pub fn get_reader(&self) -> Arc<DeviceStateReader> {
        self.deck.get_reader()
    }

    /// ボタンに単色の 120×120 画像を設定して即時反映する
    pub fn set_button_color(&self, key: u8, color: Rgb<u8>) -> Result<(), StreamDeckError> {
        let img = DynamicImage::ImageRgb8(RgbImage::from_pixel(120, 120, color));
        self.deck
            .set_button_image(key, img)
            .context("ボタン画像の設定に失敗しました")?;
        self.deck
            .flush()
            .context("ボタン画像のフラッシュに失敗しました")?;
        Ok(())
    }

    /// 全ボタンを黒 (初期状態) にクリアする
    pub fn clear_buttons(&self) -> Result<(), StreamDeckError> {
        self.deck
            .clear_all_button_images()
            .context("ボタンクリアに失敗しました")?;
        Ok(())
    }
}
