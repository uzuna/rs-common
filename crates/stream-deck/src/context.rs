//! DBus ウィンドウコンテキストによる SSH ホスト自動遷移モジュール。

use std::time::{Duration, Instant};

use dbus::arg::RefArg;
use dbus::blocking::{Connection, Proxy};
use tracing::{debug, info, warn};

use crate::runtime::RuntimeConfig;
use crate::state::PageState;

pub const DBUS_SERVICE_ENV: &str = "STREAM_DECK_DBUS_SERVICE";
pub const DBUS_PATH_ENV: &str = "STREAM_DECK_DBUS_PATH";
pub const DBUS_INTERFACE_ENV: &str = "STREAM_DECK_DBUS_INTERFACE";
pub const DBUS_PROPERTY_ENV: &str = "STREAM_DECK_DBUS_PROPERTY";

/// DBus ポーリング間隔 (Python 側の 500ms ポーリングに合わせる)
const DBUS_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// アクティブウィンドウ情報から SSH ホスト文脈を得る境界。
pub trait WindowContextProvider {
    fn poll_active_ssh_host(&mut self) -> Option<String>;
}

pub struct NoopWindowContextProvider;

impl WindowContextProvider for NoopWindowContextProvider {
    fn poll_active_ssh_host(&mut self) -> Option<String> {
        None
    }
}

/// 環境差分を吸収するため、取得元 DBus プロパティは環境変数で指定する。
/// プロパティ値に `rs-common:ssh:<host>` が含まれる場合に `<host>` を抽出する。
pub struct DbusWindowContextProvider {
    connection: Connection,
    pub service: String,
    pub path: String,
    pub interface: String,
    pub property: String,
    last_host: Option<String>,
    next_poll: Instant,
}

impl DbusWindowContextProvider {
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        let service = std::env::var(DBUS_SERVICE_ENV).ok();
        let path = std::env::var(DBUS_PATH_ENV).ok();
        let interface = std::env::var(DBUS_INTERFACE_ENV).ok();
        let property = std::env::var(DBUS_PROPERTY_ENV).ok();

        let Some(service) = service else {
            return Ok(None);
        };
        let Some(path) = path else {
            return Ok(None);
        };
        let Some(interface) = interface else {
            return Ok(None);
        };
        let Some(property) = property else {
            return Ok(None);
        };

        let connection = Connection::new_session()
            .map_err(|e| anyhow::anyhow!("DBus セッションバス接続に失敗しました: {e}"))?;

        Ok(Some(Self {
            connection,
            service,
            path,
            interface,
            property,
            last_host: None,
            next_poll: Instant::now(),
        }))
    }

    fn proxy(&self) -> Proxy<'_, &Connection> {
        self.connection.with_proxy(
            self.service.as_str(),
            self.path.as_str(),
            Duration::from_millis(80),
        )
    }

    fn fetch_context_text(&self) -> anyhow::Result<String> {
        let proxy = self.proxy();
        let (value,): (dbus::arg::Variant<Box<dyn RefArg + 'static>>,) = proxy
            .method_call(
                "org.freedesktop.DBus.Properties",
                "Get",
                (self.interface.as_str(), self.property.as_str()),
            )
            .map_err(|e| anyhow::anyhow!("DBus Properties.Get 呼び出しに失敗しました: {e}"))?;

        refarg_to_string(value.0.as_ref())
            .ok_or_else(|| anyhow::anyhow!("DBus プロパティ値を文字列へ変換できませんでした"))
    }
}

impl WindowContextProvider for DbusWindowContextProvider {
    fn poll_active_ssh_host(&mut self) -> Option<String> {
        if Instant::now() < self.next_poll {
            return None;
        }
        self.next_poll = Instant::now() + DBUS_POLL_INTERVAL;

        let text = match self.fetch_context_text() {
            Ok(text) => text,
            Err(e) => {
                debug!(err = %e, "DBus コンテキスト取得に失敗");
                return None;
            }
        };

        let host = parse_ssh_host_from_context(&text);

        if host.is_none() {
            self.last_host = None;
            return None;
        }

        let host = host.unwrap();
        if self.last_host.as_ref() == Some(&host) {
            return None;
        }

        self.last_host = Some(host.clone());
        Some(host)
    }
}

fn refarg_to_string(value: &dyn RefArg) -> Option<String> {
    if let Some(s) = value.as_str() {
        return Some(s.to_string());
    }
    if let Some(i) = value.as_i64() {
        return Some(i.to_string());
    }
    if let Some(u) = value.as_u64() {
        return Some(u.to_string());
    }
    None
}

pub fn parse_ssh_host_from_context(text: &str) -> Option<String> {
    let marker = "rs-common:ssh:";
    let start = text.find(marker)? + marker.len();
    let rest = &text[start..];
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '\u{7}' || c == '\u{1b}')
        .unwrap_or(rest.len());
    let host = rest[..end].trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

pub fn build_window_context_provider() -> Box<dyn WindowContextProvider> {
    match DbusWindowContextProvider::from_env() {
        Ok(Some(provider)) => {
            info!(
                service = %provider.service,
                path = %provider.path,
                interface = %provider.interface,
                property = %provider.property,
                "DBus WindowContextProvider を有効化"
            );
            Box::new(provider)
        }
        Ok(None) => {
            info!(
                "DBus WindowContextProvider は無効（環境変数未設定）: {} {} {} {}",
                DBUS_SERVICE_ENV, DBUS_PATH_ENV, DBUS_INTERFACE_ENV, DBUS_PROPERTY_ENV
            );
            Box::new(NoopWindowContextProvider)
        }
        Err(e) => {
            warn!(err = %e, "DBus WindowContextProvider 初期化失敗のため no-op で継続");
            Box::new(NoopWindowContextProvider)
        }
    }
}

pub fn resolve_ssh_host_target_page(config: &RuntimeConfig, host: &str) -> Option<String> {
    for page in &config.pages {
        for item in &page.items {
            if let crate::config::PageItemConfig::SshConnect {
                host: item_host, ..
            } = item
            {
                if item_host == host {
                    return Some(page.id.clone());
                }
            }
        }
    }
    None
}

pub fn apply_context_auto_transition(
    config: &RuntimeConfig,
    page_state: &mut PageState,
    provider: &mut dyn WindowContextProvider,
) -> bool {
    let Some(host) = provider.poll_active_ssh_host() else {
        return false;
    };

    let Some(target_page) = resolve_ssh_host_target_page(config, &host) else {
        debug!(host = %host, "自動遷移対象外のホスト");
        return false;
    };

    if page_state.current_page_id == target_page {
        return false;
    }

    let from = page_state.current_page_id.clone();
    page_state.set_context_page(target_page.clone());
    info!(host = %host, from = %from, to = %target_page, "コンテキスト自動遷移");
    true
}
