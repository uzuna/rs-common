//! HardwareManager: Stream Deck+ デバイスへのHIDアクセスを管理するモジュール

use anyhow::{Context, Result};
use elgato_streamdeck::{images::convert_image_with_format, list_devices, new_hidapi, StreamDeck};
use image::DynamicImage;

use crate::error::StreamDeckError;

/// デバイスへのHID接続と画像転送を管理する
pub struct HardwareManager {
    deck: StreamDeck,
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

        let deck = StreamDeck::connect(&hid, kind, &serial)
            .context("Stream Deck+ への接続に失敗しました")?;

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
}
