//! ページ状態・SSH セッション管理モジュール。

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use tracing::info;

use crate::action::{build_ssh_shell_command, compose_terminal_launch};
use crate::config::{ActionConfig, SetValueOperation};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase0ActionResultKind {
    Success,
    Rejected,
}

pub struct Phase0ActionApplyOutcome {
    pub result: Phase0ActionResultKind,
    pub feedback: String,
}

enum Phase0ActionState {
    Transition {
        states: Vec<String>,
        current_idx: usize,
    },
    SetValue {
        value: i32,
        min: i32,
        max: i32,
        step: i32,
        operation: SetValueOperation,
        set_value: Option<i32>,
        wrap: bool,
    },
}

#[derive(Default)]
pub struct Phase0ActionRuntime {
    states: HashMap<String, Phase0ActionState>,
}

impl Phase0ActionRuntime {
    pub fn from_config(config: &RuntimeConfig) -> Self {
        let mut states = HashMap::new();

        for action in &config.actions {
            match action {
                ActionConfig::Transition {
                    id,
                    states: transition_states,
                    initial_state,
                } => {
                    if transition_states.is_empty() {
                        continue;
                    }
                    let current_idx = initial_state
                        .as_ref()
                        .and_then(|initial| transition_states.iter().position(|s| s == initial))
                        .unwrap_or(0);
                    states.insert(
                        id.clone(),
                        Phase0ActionState::Transition {
                            states: transition_states.clone(),
                            current_idx,
                        },
                    );
                }
                ActionConfig::SetValue {
                    id,
                    min,
                    max,
                    initial,
                    step,
                    operation,
                    set_value,
                    wrap,
                } => {
                    let initial_value = initial.unwrap_or(*min).clamp(*min, *max);
                    states.insert(
                        id.clone(),
                        Phase0ActionState::SetValue {
                            value: initial_value,
                            min: *min,
                            max: *max,
                            step: *step,
                            operation: *operation,
                            set_value: *set_value,
                            wrap: *wrap,
                        },
                    );
                }
            }
        }

        Self { states }
    }

    pub fn apply(&mut self, action_id: &str) -> Phase0ActionApplyOutcome {
        let Some(state) = self.states.get_mut(action_id) else {
            return Phase0ActionApplyOutcome {
                result: Phase0ActionResultKind::Rejected,
                feedback: format!("{action_id}:rejected"),
            };
        };

        match state {
            Phase0ActionState::Transition {
                states,
                current_idx,
            } => {
                *current_idx = (*current_idx + 1) % states.len();
                Phase0ActionApplyOutcome {
                    result: Phase0ActionResultKind::Success,
                    feedback: states[*current_idx].clone(),
                }
            }
            Phase0ActionState::SetValue {
                value,
                min,
                max,
                step,
                operation,
                set_value,
                wrap,
            } => {
                let next_value = match operation {
                    SetValueOperation::Increment => {
                        let candidate = *value + *step;
                        if *wrap && candidate > *max {
                            *min
                        } else {
                            candidate.clamp(*min, *max)
                        }
                    }
                    SetValueOperation::Decrement => {
                        let candidate = *value - *step;
                        if *wrap && candidate < *min {
                            *max
                        } else {
                            candidate.clamp(*min, *max)
                        }
                    }
                    SetValueOperation::Set => set_value.unwrap_or(*value).clamp(*min, *max),
                };
                *value = next_value;

                Phase0ActionApplyOutcome {
                    result: Phase0ActionResultKind::Success,
                    feedback: next_value.to_string(),
                }
            }
        }
    }

    pub fn feedback_text(&self, action_id: &str) -> Option<String> {
        self.states.get(action_id).map(|state| match state {
            Phase0ActionState::Transition {
                states,
                current_idx,
            } => states[*current_idx].clone(),
            Phase0ActionState::SetValue { value, .. } => value.to_string(),
        })
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
    pub action_state_changed: bool,
    /// エンコーダ操作によるページ遷移が発生した場合、LCD を即座に更新する。
    /// ボタン操作は 1 秒ティックで十分なため `false` のままにする。
    pub lcd_needs_update: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime_with_actions(actions: Vec<ActionConfig>) -> RuntimeConfig {
        RuntimeConfig {
            sections: vec![],
            home_page_id: "home".to_string(),
            pages: vec![],
            actions,
            display: crate::display::dto::DisplayConfigDto::default(),
            watch_targets: vec![],
            podman: None,
        }
    }

    #[test]
    fn test_phase0_transition_cycles_states() {
        let config = runtime_with_actions(vec![ActionConfig::Transition {
            id: "act_mode".to_string(),
            states: vec![
                "idle".to_string(),
                "active".to_string(),
                "paused".to_string(),
            ],
            initial_state: Some("idle".to_string()),
        }]);
        let mut runtime = Phase0ActionRuntime::from_config(&config);

        assert_eq!(runtime.feedback_text("act_mode").as_deref(), Some("idle"));

        let got1 = runtime.apply("act_mode");
        assert_eq!(got1.result, Phase0ActionResultKind::Success);
        assert_eq!(got1.feedback, "active");

        let got2 = runtime.apply("act_mode");
        assert_eq!(got2.result, Phase0ActionResultKind::Success);
        assert_eq!(got2.feedback, "paused");

        let got3 = runtime.apply("act_mode");
        assert_eq!(got3.result, Phase0ActionResultKind::Success);
        assert_eq!(got3.feedback, "idle");
    }

    #[test]
    fn test_phase0_set_value_increment_wrap() {
        let config = runtime_with_actions(vec![ActionConfig::SetValue {
            id: "act_value_up".to_string(),
            min: 0,
            max: 100,
            initial: Some(90),
            step: 20,
            operation: SetValueOperation::Increment,
            set_value: None,
            wrap: true,
        }]);
        let mut runtime = Phase0ActionRuntime::from_config(&config);

        let got = runtime.apply("act_value_up");
        assert_eq!(got.result, Phase0ActionResultKind::Success);
        assert_eq!(got.feedback, "0");
    }

    #[test]
    fn test_phase0_set_value_decrement_clamp() {
        let config = runtime_with_actions(vec![ActionConfig::SetValue {
            id: "act_value_down".to_string(),
            min: 0,
            max: 100,
            initial: Some(10),
            step: 20,
            operation: SetValueOperation::Decrement,
            set_value: None,
            wrap: false,
        }]);
        let mut runtime = Phase0ActionRuntime::from_config(&config);

        let got = runtime.apply("act_value_down");
        assert_eq!(got.result, Phase0ActionResultKind::Success);
        assert_eq!(got.feedback, "0");
    }

    #[test]
    fn test_phase0_set_value_set_operation() {
        let config = runtime_with_actions(vec![ActionConfig::SetValue {
            id: "act_set".to_string(),
            min: 0,
            max: 100,
            initial: Some(10),
            step: 1,
            operation: SetValueOperation::Set,
            set_value: Some(50),
            wrap: false,
        }]);
        let mut runtime = Phase0ActionRuntime::from_config(&config);

        let got = runtime.apply("act_set");
        assert_eq!(got.result, Phase0ActionResultKind::Success);
        assert_eq!(got.feedback, "50");
    }

    #[test]
    fn test_phase0_rejects_unknown_action_id() {
        let config = runtime_with_actions(vec![]);
        let mut runtime = Phase0ActionRuntime::from_config(&config);

        let got = runtime.apply("missing");
        assert_eq!(got.result, Phase0ActionResultKind::Rejected);
        assert!(got.feedback.contains("rejected"));
    }
}
