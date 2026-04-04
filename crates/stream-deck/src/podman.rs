//! Podman REST API クライアント（Unix ドメインソケット経由、同期 HTTP/1.0）
//!
//! `PodmanClient` は `std::os::unix::net::UnixStream` で Podman ソケットに接続し、
//! 生の HTTP/1.0 リクエストを送信することで外部クレートへの依存を最小化している。
//! HTTP/1.0 はチャンク転送を使わないため、サーバが接続を閉じたら読み取り完了となる。

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use serde::Deserialize;

/// Podman REST API クライアント。
pub struct PodmanClient {
    socket_path: String,
}

/// Unix ソケット HTTP 読み取りのタイムアウト。
const HTTP_TIMEOUT: Duration = Duration::from_secs(5);

impl PodmanClient {
    pub fn new(socket_path: &str) -> Self {
        Self {
            socket_path: socket_path.to_owned(),
        }
    }

    /// HTTP/1.0 GET リクエストを送信し、ボディ文字列を返す。
    fn http_get(&self, path: &str) -> anyhow::Result<String> {
        let mut stream = UnixStream::connect(&self.socket_path).map_err(|e| {
            anyhow::anyhow!("Podman ソケットへの接続失敗 ({}): {e}", self.socket_path)
        })?;
        stream.set_read_timeout(Some(HTTP_TIMEOUT))?;
        stream.set_write_timeout(Some(HTTP_TIMEOUT))?;

        let request = format!(
            "GET {} HTTP/1.0\r\nHost: localhost\r\nAccept: application/json\r\n\r\n",
            path
        );
        stream.write_all(request.as_bytes())?;

        let mut buf = Vec::new();
        stream.read_to_end(&mut buf)?;
        let response = String::from_utf8_lossy(&buf);

        // "\r\n\r\n" 以降がボディ
        if let Some(pos) = response.find("\r\n\r\n") {
            Ok(response[pos + 4..].to_string())
        } else {
            anyhow::bail!(
                "HTTP レスポンスのヘッダー区切りが見つかりません (先頭200文字: {})",
                &response[..200.min(response.len())]
            )
        }
    }

    /// コンテナ一覧を取得する（停止中を含む）。
    pub fn load_entries(&self) -> anyhow::Result<Vec<PodmanEntry>> {
        let body = self.http_get("/v4.0.0/libpod/containers/json?all=true")?;
        serde_json::from_str::<Vec<ContainerJson>>(&body)
            .map_err(|e| {
                anyhow::anyhow!(
                    "コンテナ一覧 JSON のパース失敗: {e} (先頭200文字: {})",
                    &body[..200.min(body.len())]
                )
            })
            .map(|list| list.into_iter().map(PodmanEntry::from).collect())
    }

    /// 全コンテナの統計を取得する（実行中コンテナのみデータあり）。
    pub fn load_stats(&self) -> anyhow::Result<Vec<PodmanStatEntry>> {
        let body = self.http_get("/v4.0.0/libpod/containers/stats?stream=false")?;
        let wrapper: StatsWrapper = serde_json::from_str(&body).map_err(|e| {
            anyhow::anyhow!(
                "コンテナ統計 JSON のパース失敗: {e} (先頭200文字: {})",
                &body[..200.min(body.len())]
            )
        })?;
        Ok(wrapper
            .stats
            .into_iter()
            .map(PodmanStatEntry::from)
            .collect())
    }
}

// ── 公開データ型 ──────────────────────────────────────────────────

/// Podman コンテナエントリ（一覧取得結果）。
#[derive(Debug, Clone)]
pub struct PodmanEntry {
    pub id: String,
    pub names: Vec<String>,
    pub state: String,
    pub status: String,
}

impl PodmanEntry {
    /// 表示用ラベル。names[0] 優先、なければ短縮 ID（12文字）。
    pub fn display_label(&self) -> String {
        self.names
            .first()
            .filter(|n| !n.is_empty())
            .map(|n| n.trim_start_matches('/').to_string())
            .unwrap_or_else(|| self.id[..12.min(self.id.len())].to_string())
    }
}

/// Podman コンテナ統計エントリ（CPU 使用率）。
#[derive(Debug, Clone)]
pub struct PodmanStatEntry {
    pub id: String,
    /// CPU 使用率 (%)。Podman が計算済みの値を使う。
    pub cpu_percent: f32,
}

// ── Podman API JSON デシリアライズ用構造体 ────────────────────────

#[derive(Deserialize)]
struct ContainerJson {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Names", default)]
    names: Vec<String>,
    #[serde(rename = "State")]
    state: String,
    #[serde(rename = "Status")]
    status: String,
}

impl From<ContainerJson> for PodmanEntry {
    fn from(c: ContainerJson) -> Self {
        PodmanEntry {
            id: c.id,
            names: c.names,
            state: c.state,
            status: c.status,
        }
    }
}

#[derive(Deserialize)]
struct StatsWrapper {
    #[serde(rename = "Stats", default)]
    stats: Vec<ContainerStatJson>,
}

#[derive(Deserialize)]
struct ContainerStatJson {
    #[serde(rename = "ContainerID")]
    id: String,
    #[serde(rename = "CPU")]
    cpu: f64,
}

impl From<ContainerStatJson> for PodmanStatEntry {
    fn from(s: ContainerStatJson) -> Self {
        PodmanStatEntry {
            id: s.id,
            cpu_percent: s.cpu as f32,
        }
    }
}

// ── テスト ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 正常系: コンテナ一覧 JSON のパース
    #[test]
    fn test_parse_container_list() {
        let json = r#"[
            {"Id":"abc123def456","Names":["/myapp"],"State":"running","Status":"Up 2 hours"},
            {"Id":"deadbeef0000","Names":[],"State":"exited","Status":"Exited (0) 10 minutes ago"}
        ]"#;
        let entries: Vec<ContainerJson> = serde_json::from_str(json).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].state, "running");
        assert_eq!(entries[1].state, "exited");
    }

    /// 正常系: display_label は names[0] 優先
    #[test]
    fn test_display_label_uses_name() {
        let entry = PodmanEntry {
            id: "abc123def456".to_string(),
            names: vec!["/myapp".to_string()],
            state: "running".to_string(),
            status: "Up 2 hours".to_string(),
        };
        assert_eq!(entry.display_label(), "myapp");
    }

    /// 正常系: display_label は names 空なら短縮 ID
    #[test]
    fn test_display_label_fallback_to_id() {
        let entry = PodmanEntry {
            id: "abc123def456xxxx".to_string(),
            names: vec![],
            state: "exited".to_string(),
            status: "Exited".to_string(),
        };
        assert_eq!(entry.display_label(), "abc123def456");
    }

    /// 正常系: 統計 JSON のパース
    #[test]
    fn test_parse_stats() {
        let json = r#"{"Stats":[
            {"ContainerID":"abc123","CPU":1.23},
            {"ContainerID":"def456","CPU":0.0}
        ]}"#;
        let wrapper: StatsWrapper = serde_json::from_str(json).unwrap();
        assert_eq!(wrapper.stats.len(), 2);
        assert!((wrapper.stats[0].cpu - 1.23).abs() < 1e-6);
    }

    /// 正常系: Stats フィールドなし (空) でも panic しない
    #[test]
    fn test_parse_stats_empty() {
        let json = r#"{"Stats":[]}"#;
        let wrapper: StatsWrapper = serde_json::from_str(json).unwrap();
        assert!(wrapper.stats.is_empty());
    }

    /// 異常系: 壊れた JSON は Err を返す (実際の parse 失敗確認)
    #[test]
    fn test_parse_container_list_broken_json() {
        let result = serde_json::from_str::<Vec<ContainerJson>>("not json");
        assert!(result.is_err());
    }
}
