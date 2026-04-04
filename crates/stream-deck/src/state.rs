//! ページ状態・SSH セッション管理モジュール。

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use tracing::info;

use crate::action::{build_ssh_shell_command, compose_terminal_launch};
use crate::runtime::{fallback_page_id, page_exists, RuntimeConfig};

/// 現在ページ ID と遷移履歴を保持するページ状態機械。
pub struct PageState {
    pub current_page_id: String,
    pub history: Vec<String>,
}

impl PageState {
    pub fn new(config: &RuntimeConfig) -> Self {
        let current_page_id = fallback_page_id(config).unwrap_or_default();
        Self {
            current_page_id,
            history: Vec::new(),
        }
    }

    /// 設定リロード後にページ ID が存在するか確認し、なければホームへ戻す。
    pub fn reconcile(&mut self, config: &RuntimeConfig) {
        if page_exists(config, &self.current_page_id) {
            return;
        }
        self.current_page_id = fallback_page_id(config).unwrap_or_default();
        self.history.clear();
    }

    pub fn navigate_to(&mut self, target: String) {
        self.history.push(self.current_page_id.clone());
        self.current_page_id = target;
    }

    pub fn back(&mut self) {
        if let Some(prev) = self.history.pop() {
            self.current_page_id = prev;
        }
    }

    /// 自動コンテキスト遷移は履歴へ push せず現在ページのみ差し替える。
    pub fn set_context_page(&mut self, target: String) {
        self.current_page_id = target;
    }
}

/// host -> 起動済みプロセス ID のレジストリ。セッション再利用とウィンドウフォーカスを管理する。
#[derive(Default)]
pub struct SshSessionRegistry {
    sessions: HashMap<String, u32>,
}

impl SshSessionRegistry {
    /// `/proc/<pid>` の存在でプロセス生存を確認する。
    fn is_alive(pid: u32) -> bool {
        Path::new(&format!("/proc/{pid}")).exists()
    }

    /// ウィンドウタイトルで既存セッションへのフォーカスを試みる。
    /// `wmctrl -a` → `xdotool search --name windowactivate` の順にフォールバックする。
    fn try_focus(title: &str) -> bool {
        if Command::new("wmctrl")
            .args(["-a", title])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return true;
        }
        Command::new("xdotool")
            .args(["search", "--name", title, "windowactivate"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// 既存セッションがあればフォーカス、なければ新規起動する。
    pub fn connect(
        &mut self,
        host: &str,
        terminal: &[String],
        ssh_template: &str,
    ) -> anyhow::Result<()> {
        let session_title = format!("rs-common:ssh:{host}");

        if let Some(&pid) = self.sessions.get(host) {
            if Self::is_alive(pid) {
                if Self::try_focus(&session_title) {
                    info!(host, pid, "既存 SSH ウィンドウへフォーカスしました");
                } else {
                    info!(
                        host,
                        pid, "既存 SSH セッションを再利用します（フォーカスは未保証）"
                    );
                }
                return Ok(());
            }
            // PID が死んでいる場合はレジストリから削除して新規起動へ
            self.sessions.remove(host);
        }

        let shell_command = build_ssh_shell_command(host, ssh_template, &session_title);
        let (program, args) = compose_terminal_launch(terminal, &shell_command, &session_title)?;
        let child = Command::new(&program).args(&args).spawn().map_err(|e| {
            anyhow::anyhow!("SSH 接続用端末の起動に失敗しました program={program}: {e}")
        })?;

        let pid = child.id();
        self.sessions.insert(host.to_string(), pid);
        info!(host, pid, program = %program, "SSH セッションを新規起動しました");
        Ok(())
    }
}

/// 1回の入力ポーリングで発生した副作用の集約結果。
/// 呼び出し側はこのフラグを使ってデバイス反映を最小化する。
#[derive(Default)]
pub struct InputUpdateOutcome {
    pub brightness_changed: bool,
    pub notification_state_changed: bool,
    pub page_state_changed: bool,
    /// エンコーダ操作によるページ遷移が発生した場合、LCD を即座に更新する。
    /// ボタン操作は 1 秒ティックで十分なため `false` のままにする。
    pub lcd_needs_update: bool,
}
