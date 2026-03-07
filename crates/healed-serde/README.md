# healed-serde

堅牢なIoT向け永続化ライブラリ

## 概要

`healed-serde` は、ビット腐敗（ビット反転）や書き込み中の電源断が発生しやすい環境（IoTデバイスなど）において、データを安全に保存・復元するためのRustライブラリです。

## 主な機能

*   **自動修復**: SECDED（Single Error Correction, Double Error Detection）ハミング符号を用いて、1ビットのエラーを自動的に訂正し、2ビットのエラーを検出します。
*   **電源断保護**: 3つのファイルスロットを使用したローリングアップデート戦略により、書き込み中の電源断によるデータ破損を防ぎます。常に最新の健全なデータを読み込みます。
*   **整合性チェック**: CRC32によるデータ整合性の検証を行います。
*   **柔軟な保護レベル**: データの重要度やサイズに応じて、保護レベル（オーバーヘッドと保護性能のトレードオフ）を選択可能です。
*   **抽象化されたストレージ**: `StorageBackend` トレイトを実装することで、ファイルシステム以外のバックエンド（KVS、mmapなど）にも対応可能です。

## 使用方法

`ReliableVault` 構造体を使用してデータの保存と読み込みを行います。保存対象のデータ構造体は `serde::Serialize` および `serde::Deserialize` を実装している必要があります。

```rust
use healed_serde::vault::ReliableVault;
use healed_serde::ProtectionLevel;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct DeviceConfig {
    id: u32,
    name: String,
    settings: Vec<u8>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 保存先ディレクトリとファイル名のベースを指定
    // 例: ./data/device_config.0, ./data/device_config.1, ...
    let vault = ReliableVault::new_with_fs("./data", "device_config");

    let config = DeviceConfig {
        id: 12345,
        name: "sensor-node-01".to_string(),
        settings: vec![0x01, 0x02, 0x03],
    };

    // データを保存 (ProtectionLevel::Medium はバランス型)
    vault.save(&config, ProtectionLevel::Medium)?;

    // データを読み込み
    // 破損がある場合は自動的に修復、または過去の健全なバックアップから復元されます
    let loaded_config: DeviceConfig = vault.load()?;

    println!("Loaded: {:?}", loaded_config);
    Ok(())
}
```

## 保護レベル (ProtectionLevel)

*   `High`: 8bitデータごとにECCを付与。オーバーヘッド大、保護性能高。
*   `Medium`: 32bitデータごとにECCを付与。バランス型。
*   `Low`: 64bitデータごとにECCを付与。オーバーヘッド小。

## ファイル構成とローリング戦略

`ReliableVault` は、データの整合性を保つために3つのファイルスロットを使用します。

*   **ファイル名**: 指定されたベース名に `.0`, `.1`, `.2` のサフィックスが付与されます（例: `data.0`, `data.1`, `data.2`）。
*   **書き込み**: 常に最も古いシーケンス番号を持つスロット（または未作成のスロット）を上書きします。これにより、書き込み中に電源断が発生しても、他の2つのスロットには過去の有効なデータが残ります。
*   **読み込み**: 全てのスロットをスキャンし、破損していないデータの中で最も新しいシーケンス番号を持つものを自動的に選択します。

### ファイルフォーマット

各ファイルは以下の構造を持つバイナリ形式で保存されます。

1.  **Primary Header (16 bytes)**: メタデータ（シーケンス番号、保護レベル、ペイロード長）。SECDEDで保護されており、1ビットエラーを自動修復可能。
2.  **Secondary Header (16 bytes)**: Primary Headerのバックアップコピー。
3.  **Payload**: ユーザーデータ。指定された `ProtectionLevel` に基づいてブロック分割され、ECCエンコードされています。
4.  **Footer (8 bytes)**: 全体のCRC32チェックサムとシーケンス番号の検証用データ。
