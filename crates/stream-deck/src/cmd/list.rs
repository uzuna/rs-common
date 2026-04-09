//! list サブコマンド: 接続中の Stream Deck デバイスを一覧表示する

use elgato_streamdeck::{list_devices, new_hidapi, StreamDeck};

/// 接続中の全 Stream Deck デバイス情報を標準出力に表示する
pub fn run() -> anyhow::Result<()> {
    let hid = new_hidapi()?;
    let devices = list_devices(&hid);

    if devices.is_empty() {
        println!("Stream Deck デバイスが見つかりませんでした。");
        println!("ヒント: `stream-deck diagnose` で接続環境を確認してください。");
        return Ok(());
    }

    println!("検出された Stream Deck デバイス ({} 台):", devices.len());
    println!(
        "{:<4} {:<22} {:<22} {:<10} ファームウェア",
        "#", "種別", "製品名", "シリアル"
    );
    println!("{}", "─".repeat(80));

    for (i, (kind, serial)) in devices.iter().enumerate() {
        // デバイスに接続して詳細情報を取得（失敗した場合は "-" を表示）
        match StreamDeck::connect(&hid, *kind, serial) {
            Ok(deck) => {
                let product = deck.product().unwrap_or_else(|_| "-".to_string());
                let firmware = deck.firmware_version().unwrap_or_else(|_| "-".to_string());
                let (w, h) = kind.lcd_strip_size().unwrap_or((0, 0));
                let kind_str = format!("{kind:?}");
                println!(
                    "{:<4} {:<22} {:<22} {:<10} {}",
                    i + 1,
                    kind_str,
                    product,
                    serial,
                    firmware,
                );
                if w > 0 {
                    println!("     └ LCD ストリップ: {w}x{h} px");
                }
            }
            Err(e) => {
                println!("{:<4} {:<22} (接続失敗: {e})", i + 1, format!("{kind:?}"));
            }
        }
    }

    Ok(())
}
