//! SharedObject プラグインを `plugin_base::PluginHandle` で包むアダプタ。
//!
//! [`SoPluginHandle`] は `PluginRouter` を所有し、
//! `hello()` / `add()` を HTTP ABI 経由でプラグインへ委譲する。
//!
//! # エンドポイントの対応
//!
//! | `PluginHandle` メソッド | SO プラグイン                                       |
//! | :---------------------- | :-------------------------------------------------- |
//! | `hello()`               | example-plugin `GET {hello_prefix}/hello`           |
//! | `add(a, b, _)`          | sample-plugin `POST {add_prefix}/add` (loop_count 無視) |

use std::path::Path;

use anyhow::Context;
use plugin_base::PluginHandle;

use crate::plugin_manager::PluginRouter;

/// SharedObject プラグインを `PluginHandle` トレイト経由で呼び出すアダプタ。
pub struct SoPluginHandle {
    router: PluginRouter,
    hello_path: String,
    add_path: String,
}

impl SoPluginHandle {
    /// example-plugin と sample-plugin をロードして `SoPluginHandle` を生成する。
    ///
    /// - `hello_plugin_path`: `hello()` に対応するプラグイン（example-plugin）の .so パス
    /// - `hello_prefix`: そのマウントプレフィックス（例: `"/api"`）
    /// - `add_plugin_path`: `add()` に対応するプラグイン（sample-plugin）の .so パス
    /// - `add_prefix`: そのマウントプレフィックス（例: `"/sample"`）
    pub fn load(
        hello_plugin_path: &Path,
        hello_prefix: &str,
        add_plugin_path: &Path,
        add_prefix: &str,
    ) -> anyhow::Result<Self> {
        let mut router = PluginRouter::default();
        router
            .load(hello_prefix, hello_plugin_path)
            .with_context(|| {
                format!(
                    "hello プラグインのロードに失敗: {}",
                    hello_plugin_path.display()
                )
            })?;
        router.load(add_prefix, add_plugin_path).with_context(|| {
            format!(
                "add プラグインのロードに失敗: {}",
                add_plugin_path.display()
            )
        })?;
        Ok(Self {
            router,
            hello_path: format!("{hello_prefix}/hello"),
            add_path: format!("{add_prefix}/add"),
        })
    }
}

impl PluginHandle for SoPluginHandle {
    /// `GET {hello_prefix}/hello` を呼び出す。
    fn hello(&mut self) -> anyhow::Result<()> {
        let resp = self.router.handle_ref("GET", &self.hello_path, "", &[]);
        anyhow::ensure!(
            resp.status == 200,
            "hello 失敗: status={}, body={}",
            resp.status,
            String::from_utf8_lossy(resp.body.as_slice())
        );
        Ok(())
    }

    /// `POST {add_prefix}/add` を呼び出す（`loop_count` は無視）。
    fn add(&mut self, a: i32, b: i32, _loop_count: i32) -> anyhow::Result<i32> {
        let body = format!(r#"{{"a":{},"b":{}}}"#, a, b);
        let resp = self
            .router
            .handle_ref("POST", &self.add_path, "", body.as_bytes());
        anyhow::ensure!(
            resp.status == 200,
            "add 失敗: status={}, body={}",
            resp.status,
            String::from_utf8_lossy(resp.body.as_slice())
        );
        let v: serde_json::Value = serde_json::from_slice(resp.body.as_slice())
            .context("add レスポンスの JSON パース失敗")?;
        let result = v
            .get("result")
            .and_then(|r| r.as_i64())
            .context("add レスポンスに数値の result フィールドが存在しない")?;
        Ok(result as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct SoCase {
        name: &'static str,
        hello_path: &'static str,
        add_path: &'static str,
    }

    fn so_plugin_path(name: &str) -> std::path::PathBuf {
        // ワークスペースの target/debug を参照する
        // クレート名 "safety-plugin-{name}" → "libsafety_plugin_{name}.so"
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/debug")
            .join(format!("libsafety_plugin_{name}.so"))
    }

    fn skip_if_no_so(name: &str) -> Option<std::path::PathBuf> {
        let path = so_plugin_path(name);
        if !path.exists() {
            eprintln!(
                "スキップ: {} が見つかりません（{}）。先に `cargo build -p safty-plugin-{}` を実行してください",
                name,
                path.display(),
                name
            );
            None
        } else {
            Some(path)
        }
    }

    #[test]
    fn load_値域確認() {
        let Some(example_path) = skip_if_no_so("example") else {
            return;
        };
        let Some(sample_path) = skip_if_no_so("sample") else {
            return;
        };

        let handle = SoPluginHandle::load(&example_path, "/api", &sample_path, "/sample").unwrap();
        assert_eq!(handle.hello_path, "/api/hello");
        assert_eq!(handle.add_path, "/sample/add");
    }

    #[test]
    fn hello_正常系() {
        let cases = [SoCase {
            name: "example-plugin hello",
            hello_path: "/api",
            add_path: "/sample",
        }];

        for case in &cases {
            let Some(example_path) = skip_if_no_so("example") else {
                return;
            };
            let Some(sample_path) = skip_if_no_so("sample") else {
                return;
            };
            let mut handle =
                SoPluginHandle::load(&example_path, case.hello_path, &sample_path, case.add_path)
                    .expect(&format!("ケース '{}': ロード失敗", case.name));
            handle
                .hello()
                .expect(&format!("ケース '{}': hello 失敗", case.name));
        }
    }

    #[test]
    fn add_正常系() {
        let Some(example_path) = skip_if_no_so("example") else {
            return;
        };
        let Some(sample_path) = skip_if_no_so("sample") else {
            return;
        };
        let mut handle =
            SoPluginHandle::load(&example_path, "/api", &sample_path, "/sample").unwrap();

        let cases = [(11i32, 7i32, 18i32), (0, 0, 0), (-5, 3, -2)];
        for (a, b, expected) in cases {
            let result = handle.add(a, b, 1).expect(&format!("add({a}, {b}) 失敗"));
            assert_eq!(
                result, expected,
                "add({a}, {b}) = {result}, expected {expected}"
            );
        }
    }
}
