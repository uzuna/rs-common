//! WASM 方式と SharedObject 方式のプラグインを共通インターフェースで扱うクレート。
//!
//! # 目的
//!
//! 2つのプラグイン方式のパフォーマンスを同一ベンチマーク内で比較するため、
//! 共通のデータ型とトレイトを定義する。
//!
//! | トレイト         | 役割                                             |
//! | :--------------- | :----------------------------------------------- |
//! | [`PluginHandle`] | 呼び出しオーバーヘッド計測用の共通インターフェース |
//! | [`StatefulPlugin`] | ホットリロード対応の状態保存・復元インターフェース |
//!
//! # 各方式の実装マッピング
//!
//! | 操作                    | WASM (WIT 経由)          | SharedObject (HTTP ABI 経由)        |
//! | :---------------------- | :----------------------- | :---------------------------------- |
//! | `hello()`               | WIT `get-status` を呼ぶ  | `GET {prefix}/hello` を呼ぶ         |
//! | `add(a, b, loop_count)` | WIT `add(a, b, n)` を呼ぶ | `POST {prefix}/add` で `a+b` を得る |

/// センサーデータ（WIT `sensor-data` と対応）。
#[derive(Debug, Clone, PartialEq)]
pub struct SensorData {
    /// モーター負荷
    pub load: f32,
    /// モーターポジション
    pub position: f32,
    /// 追加センサー値（省略可）
    pub extra: Option<f32>,
}

/// モーター制御出力（WIT `motor-output` と対応）。
#[derive(Debug, Clone, PartialEq)]
pub struct MotorOutput {
    /// 目標位置
    pub position: f32,
    /// 出力トルク
    pub torque: f32,
}

/// プラグインの内部状態（WIT `plugin-status` と対応）。
#[derive(Debug, Clone, PartialEq)]
pub struct PluginStatus {
    /// 稼働中フラグ
    pub running: bool,
    /// エラーコード（0=正常）
    pub error_code: u32,
    /// 内部温度
    pub temperature: f32,
}

/// プラグイン呼び出しの共通インターフェース。
///
/// WASM 方式と SharedObject 方式の両方が実装し、ベンチマークで `Box<dyn PluginHandle>`
/// 経由で比較できるようにする。
pub trait PluginHandle {
    /// 呼び出しオーバーヘッド計測用の軽量操作。
    ///
    /// - **WASM 実装:** WIT `get-status` を呼び出す
    /// - **SharedObject 実装:** `GET {prefix}/hello` を呼び出す
    fn hello(&mut self) -> anyhow::Result<()>;

    /// 加算操作。
    ///
    /// - **WASM 実装:** WIT `add(a, b, loop_count)`は1回の呼び出して内部 `loop_count` 回加算する
    /// - **SharedObject 実装:** `POST {prefix}/add` で `a + b` を得る（`loop_count` は無視）
    fn add(&mut self, a: i32, b: i32, loop_count: i32) -> anyhow::Result<i32>;
}

/// 状態引き継ぎ対応プラグインの拡張インターフェース。
///
/// [`PluginHandle`] を拡張し、ホットリロード時の状態保存・復元を可能にする。
pub trait StatefulPlugin: PluginHandle {
    /// 内部状態をバイト列にシリアライズして返す。
    ///
    /// ホストはリロード前にこのメソッドを呼び、戻り値を保持する。
    fn save_state(&mut self) -> anyhow::Result<Vec<u8>>;

    /// バイト列から内部状態を復元する。
    ///
    /// ホストはリロード後の新インスタンスに前回の `save_state` 結果を渡す。
    /// 初回起動時は空スライスを渡す。
    fn load_state(&mut self, state: &[u8]) -> anyhow::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TypeCase {
        name: &'static str,
    }

    #[test]
    fn 共通型_構築確認() {
        let cases = [
            TypeCase { name: "SensorData" },
            TypeCase {
                name: "MotorOutput",
            },
            TypeCase {
                name: "PluginStatus",
            },
        ];

        for case in &cases {
            match case.name {
                "SensorData" => {
                    let s = SensorData {
                        load: 1.0,
                        position: 0.5,
                        extra: Some(0.1),
                    };
                    assert_eq!(s.load, 1.0, "SensorData.load");
                    assert_eq!(s.position, 0.5, "SensorData.position");
                    assert_eq!(s.extra, Some(0.1), "SensorData.extra");

                    let s_none = SensorData {
                        load: 0.0,
                        position: 0.0,
                        extra: None,
                    };
                    assert!(
                        s_none.extra.is_none(),
                        "SensorData.extra は None を受け入れる"
                    );
                }
                "MotorOutput" => {
                    let m = MotorOutput {
                        position: 2.0,
                        torque: 0.8,
                    };
                    assert_eq!(m.position, 2.0, "MotorOutput.position");
                    assert_eq!(m.torque, 0.8, "MotorOutput.torque");
                }
                "PluginStatus" => {
                    let p = PluginStatus {
                        running: true,
                        error_code: 0,
                        temperature: 36.5,
                    };
                    assert!(p.running, "PluginStatus.running");
                    assert_eq!(p.error_code, 0, "PluginStatus.error_code");
                    assert_eq!(p.temperature, 36.5, "PluginStatus.temperature");

                    let p_err = PluginStatus {
                        running: false,
                        error_code: 1,
                        temperature: 0.0,
                    };
                    assert!(!p_err.running, "PluginStatus.running=false");
                    assert_ne!(p_err.error_code, 0, "PluginStatus.error_code != 0");
                }
                _ => unreachable!("未知の型ケース: {}", case.name),
            }
        }
    }

    #[test]
    fn 共通型_clone_eq確認() {
        let sensor = SensorData {
            load: 1.5,
            position: -0.3,
            extra: None,
        };
        assert_eq!(
            sensor.clone(),
            sensor,
            "SensorData は Clone + PartialEq を実装する"
        );

        let motor = MotorOutput {
            position: 1.0,
            torque: 0.5,
        };
        assert_eq!(
            motor.clone(),
            motor,
            "MotorOutput は Clone + PartialEq を実装する"
        );

        let status = PluginStatus {
            running: true,
            error_code: 0,
            temperature: 25.0,
        };
        assert_eq!(
            status.clone(),
            status,
            "PluginStatus は Clone + PartialEq を実装する"
        );
    }
}
