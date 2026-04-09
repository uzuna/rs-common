//! 通知管理モジュール: 受信・状態管理・ボタン LED 制御

use std::fmt;
use std::os::unix::net::UnixDatagram;
use std::path::Path;
use std::time::{Duration, Instant};

use clap::ValueEnum;
use serde::Deserialize;
use tracing::{debug, info, warn};

/// Pending 状態を既読にするまでの猶予
pub const NOTIFICATION_PENDING_TIMEOUT: Duration = Duration::from_secs(2);

/// 受信する通知 1 件の最大サイズ
const NOTIFICATION_MAX_BYTES: usize = 4096;

/// LCD に表示する通知サマリ最大文字数
const NOTIFICATION_SUMMARY_MAX_CHARS: usize = 14;

// ── ソケット受信 ──────────────────────────────────────────────────

#[derive(Debug)]
pub struct NotificationSource {
    socket: UnixDatagram,
    recv_buf: [u8; NOTIFICATION_MAX_BYTES],
}

impl NotificationSource {
    pub fn bind(path: &str) -> anyhow::Result<Self> {
        let socket_path = Path::new(path);
        if socket_path.exists() {
            std::fs::remove_file(socket_path)?;
        }
        let socket = UnixDatagram::bind(socket_path)?;
        socket.set_nonblocking(true)?;
        info!(path, "通知ソケット待受を開始");
        Ok(Self {
            socket,
            recv_buf: [0; NOTIFICATION_MAX_BYTES],
        })
    }

    pub fn try_recv(&mut self) -> anyhow::Result<Option<IncomingNotification>> {
        match self.socket.recv(&mut self.recv_buf) {
            Ok(size) => {
                let payload = std::str::from_utf8(&self.recv_buf[..size])?;
                let packet: NotificationPacket = serde_json::from_str(payload)?;
                Ok(Some(IncomingNotification {
                    summary: packet.summary,
                    body: packet.body.unwrap_or_default(),
                    action_payload: packet.action_payload,
                }))
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(err) => Err(err.into()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct NotificationPacket {
    summary: String,
    #[serde(default)]
    body: Option<String>,
    action_payload: String,
}

#[derive(Debug)]
pub struct IncomingNotification {
    pub summary: String,
    pub body: String,
    pub action_payload: String,
}

// ── 通知アイテム ──────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct NotificationItem {
    pub id: u64,
    pub summary: String,
    pub body: String,
    pub action_payload: String,
    pub created_at: Instant,
}

#[derive(Debug, Clone)]
pub enum SlotState {
    Empty,
    Unread {
        item: NotificationItem,
    },
    Pending {
        item: NotificationItem,
        deadline: Instant,
    },
}

impl SlotState {
    pub fn color(&self) -> ButtonColor {
        match self {
            SlotState::Empty => ButtonColor::Black,
            SlotState::Unread { .. } => ButtonColor::Blue,
            SlotState::Pending { .. } => ButtonColor::Yellow,
        }
    }
}

#[derive(Debug)]
pub enum NotificationPressAction {
    Execute(String),
    RevertToUnread,
}

// ── 通知状態 ───────────────────────────────────────────────────────

pub struct NotificationState {
    pub slots: [SlotState; 8],
    pub next_id: u64,
    compaction_mode: SlotCompactionMode,
}

impl NotificationState {
    pub fn new(compaction_mode: SlotCompactionMode) -> Self {
        Self {
            slots: std::array::from_fn(|_| SlotState::Empty),
            next_id: 1,
            compaction_mode,
        }
    }

    pub fn insert(&mut self, incoming: IncomingNotification, now: Instant) -> Option<usize> {
        let item = NotificationItem {
            id: self.next_id,
            summary: incoming.summary,
            body: incoming.body,
            action_payload: incoming.action_payload,
            created_at: now,
        };
        self.next_id += 1;

        if let Some((idx, _)) = self
            .slots
            .iter()
            .enumerate()
            .find(|(_, slot)| matches!(slot, SlotState::Empty))
        {
            self.slots[idx] = SlotState::Unread { item };
            return Some(idx);
        }

        let target = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| match slot {
                SlotState::Unread { item } => Some((idx, item.created_at)),
                _ => None,
            })
            .min_by_key(|(_, created_at)| *created_at)
            .map(|(idx, _)| idx);

        match target {
            Some(idx) => {
                self.slots[idx] = SlotState::Unread { item };
                Some(idx)
            }
            None => {
                warn!("通知スロットが埋まっているため破棄しました");
                None
            }
        }
    }

    pub fn unread_count(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| matches!(slot, SlotState::Unread { .. }))
            .count()
    }

    pub fn total_active_count(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| !matches!(slot, SlotState::Empty))
            .count()
    }

    pub fn on_button_down(&mut self, idx: usize, now: Instant) -> Option<NotificationPressAction> {
        if idx >= self.slots.len() {
            return None;
        }
        match &self.slots[idx] {
            SlotState::Unread { item } => {
                let payload = item.action_payload.clone();
                let moved = item.clone();
                debug!(slot = idx, notif_id = item.id, "通知アクションを実行");
                self.slots[idx] = SlotState::Pending {
                    item: moved,
                    deadline: now + NOTIFICATION_PENDING_TIMEOUT,
                };
                Some(NotificationPressAction::Execute(payload))
            }
            SlotState::Pending { item, .. } => {
                debug!(slot = idx, notif_id = item.id, "通知を未読に戻しました");
                self.slots[idx] = SlotState::Unread { item: item.clone() };
                Some(NotificationPressAction::RevertToUnread)
            }
            SlotState::Empty => None,
        }
    }

    pub fn expire_pending(&mut self, now: Instant) -> bool {
        let mut changed = false;
        for slot in &mut self.slots {
            if let SlotState::Pending { deadline, .. } = slot {
                if *deadline <= now {
                    *slot = SlotState::Empty;
                    changed = true;
                }
            }
        }
        if changed && self.compaction_mode == SlotCompactionMode::CompactLeft {
            self.compact_left();
        }
        changed
    }

    fn compact_left(&mut self) {
        let mut non_empty: Vec<SlotState> = self
            .slots
            .iter()
            .filter(|slot| !matches!(slot, SlotState::Empty))
            .cloned()
            .collect();
        non_empty.resize_with(self.slots.len(), || SlotState::Empty);
        for (idx, slot) in non_empty.into_iter().enumerate() {
            self.slots[idx] = slot;
        }
    }

    pub fn latest_item(&self) -> Option<&NotificationItem> {
        self.slots
            .iter()
            .filter_map(|slot| match slot {
                SlotState::Unread { item } | SlotState::Pending { item, .. } => Some(item),
                SlotState::Empty => None,
            })
            .max_by_key(|item| item.created_at)
    }

    pub fn overlay_text(&self) -> String {
        let active = self.total_active_count();
        if active == 0 {
            return "0".to_string();
        }
        let summary = self
            .latest_item()
            .map(|item| {
                let text = if item.summary.trim().is_empty() {
                    item.body.as_str()
                } else {
                    item.summary.as_str()
                };
                format!(
                    "#{} {}",
                    item.id,
                    truncate_chars(text, NOTIFICATION_SUMMARY_MAX_CHARS)
                )
            })
            .unwrap_or_else(|| "-".to_string());
        format!("{active} {summary}")
    }

    pub fn active_ratio(&self) -> f32 {
        self.total_active_count() as f32 / self.slots.len() as f32
    }
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    let mut chars = input.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

// ── 補助型 ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SlotCompactionMode {
    KeepGap,
    CompactLeft,
}

impl fmt::Display for SlotCompactionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            SlotCompactionMode::KeepGap => "keep-gap",
            SlotCompactionMode::CompactLeft => "compact-left",
        };
        write!(f, "{label}")
    }
}

/// 通知ボタンの表示色
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonColor {
    Black,
    Blue,
    Yellow,
}

impl ButtonColor {
    pub fn to_rgb(self) -> image::Rgb<u8> {
        match self {
            ButtonColor::Black => image::Rgb([0, 0, 0]),
            ButtonColor::Blue => image::Rgb([0, 0, 255]),
            ButtonColor::Yellow => image::Rgb([255, 200, 0]),
        }
    }
}

// ── テスト ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notification_transition_unread_pending_unread() {
        let mut state = NotificationState::new(SlotCompactionMode::KeepGap);
        let now = Instant::now();
        state.insert(
            IncomingNotification {
                summary: "summary".to_string(),
                body: "body".to_string(),
                action_payload: "vscode://file/tmp/result.csv".to_string(),
            },
            now,
        );

        let action = state.on_button_down(0, now + Duration::from_millis(1));
        assert!(matches!(action, Some(NotificationPressAction::Execute(_))));
        assert!(matches!(state.slots[0], SlotState::Pending { .. }));

        let action2 = state.on_button_down(0, now + Duration::from_millis(2));
        assert!(matches!(
            action2,
            Some(NotificationPressAction::RevertToUnread)
        ));
        assert!(matches!(state.slots[0], SlotState::Unread { .. }));
    }

    #[test]
    fn test_notification_pending_timeout_to_empty() {
        let mut state = NotificationState::new(SlotCompactionMode::KeepGap);
        let now = Instant::now();
        state.insert(
            IncomingNotification {
                summary: "summary".to_string(),
                body: "body".to_string(),
                action_payload: "vscode://file/tmp/result.csv".to_string(),
            },
            now,
        );
        state.on_button_down(0, now);

        let changed =
            state.expire_pending(now + NOTIFICATION_PENDING_TIMEOUT + Duration::from_millis(1));
        assert!(changed);
        assert!(matches!(state.slots[0], SlotState::Empty));
    }

    #[test]
    fn test_notification_replaces_oldest_unread_when_full() {
        let mut state = NotificationState::new(SlotCompactionMode::KeepGap);
        let base = Instant::now();

        for i in 0..8 {
            state.insert(
                IncomingNotification {
                    summary: format!("n{i}"),
                    body: String::new(),
                    action_payload: format!("vscode://file/tmp/n{i}.txt"),
                },
                base + Duration::from_millis(i as u64),
            );
        }

        state.insert(
            IncomingNotification {
                summary: "latest".to_string(),
                body: String::new(),
                action_payload: "vscode://file/tmp/latest.txt".to_string(),
            },
            base + Duration::from_secs(1),
        );

        let first_slot_summary = match &state.slots[0] {
            SlotState::Unread { item } => item.summary.clone(),
            _ => String::new(),
        };
        assert_eq!(first_slot_summary, "latest");
    }

    #[test]
    fn test_notification_compact_left_on_expire() {
        let mut state = NotificationState::new(SlotCompactionMode::CompactLeft);
        let base = Instant::now();

        for summary in ["a", "b", "c"] {
            state.insert(
                IncomingNotification {
                    summary: summary.to_string(),
                    body: String::new(),
                    action_payload: format!("vscode://file/tmp/{summary}"),
                },
                base + Duration::from_millis(state.next_id),
            );
        }

        state.on_button_down(1, base + Duration::from_millis(10));
        state.expire_pending(base + NOTIFICATION_PENDING_TIMEOUT + Duration::from_millis(10));

        assert!(matches!(state.slots[0], SlotState::Unread { .. }));
        assert!(matches!(state.slots[1], SlotState::Unread { .. }));
        assert!(matches!(state.slots[2], SlotState::Empty));
    }

    #[test]
    fn test_notification_keep_gap_on_expire() {
        let mut state = NotificationState::new(SlotCompactionMode::KeepGap);
        let base = Instant::now();

        for summary in ["a", "b", "c"] {
            state.insert(
                IncomingNotification {
                    summary: summary.to_string(),
                    body: String::new(),
                    action_payload: format!("vscode://file/tmp/{summary}"),
                },
                base + Duration::from_millis(state.next_id),
            );
        }

        state.on_button_down(1, base + Duration::from_millis(10));
        state.expire_pending(base + NOTIFICATION_PENDING_TIMEOUT + Duration::from_millis(10));

        assert!(matches!(state.slots[0], SlotState::Unread { .. }));
        assert!(matches!(state.slots[1], SlotState::Empty));
        assert!(matches!(state.slots[2], SlotState::Unread { .. }));
    }
}
