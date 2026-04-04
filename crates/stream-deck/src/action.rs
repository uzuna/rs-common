//! アクション実行モジュール。
//! 外部プロセス起動・シェルコマンド組み立て・SSH セッション管理を担う。

use std::path::Path;
use std::process::Command;

use tracing::info;

/// 実行境界で扱うアクション要求。
/// 通知由来の open と将来の command を同じ入口に載せる。
/// SSH 接続は SshSessionRegistry 経由で実行するため、ここには含まない。
pub enum ActionRequest {
    OpenTarget(String),
    Command {
        program: String,
        args: Vec<String>,
    },
    PodmanLogs {
        container_id: String,
        program: String,
        args: Vec<String>,
    },
}

pub fn resolve_default_terminal_program() -> String {
    let output = Command::new("readlink")
        .arg("-e")
        .arg("/usr/bin/x-terminal-emulator")
        .output();

    if let Ok(out) = output {
        if out.status.success() {
            let candidate = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !candidate.is_empty() {
                return candidate;
            }
        }
    }

    "/usr/bin/x-terminal-emulator".to_string()
}

/// シェルの単一引用符コンテキスト内の文字列をエスケープする。
/// `'raw'` の形式で使用する前提で、内部の `'` を `'"'"'` に置換する。
pub fn shell_escape_single_quoted(raw: &str) -> String {
    raw.replace('\'', "'\"'\"'")
}

/// シェル引数として安全に使用できるよう単一引用符で囲む。
/// 内部の `'` は `'"'"'` でエスケープされる。
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

pub fn build_ssh_shell_command(host: &str, ssh_template: &str, title: &str) -> String {
    // host を shell_quote で保護してからテンプレートに代入する。
    // これにより host に `;` や `&&` が含まれても シェルインジェクションを防ぐ。
    let quoted_host = shell_quote(host);
    let ssh_command = ssh_template.replace("{host}", &quoted_host);
    let escaped_title = shell_escape_single_quoted(title);
    format!(
        "printf '\\033]0;{}\\007'; exec {}",
        escaped_title, ssh_command
    )
}

pub fn compose_terminal_launch(
    terminal: &[String],
    shell_command: &str,
    title: &str,
) -> anyhow::Result<(String, Vec<String>)> {
    let (program, mut args) = if terminal.is_empty() {
        (resolve_default_terminal_program(), Vec::new())
    } else {
        (terminal[0].clone(), terminal[1..].to_vec())
    };

    if program.trim().is_empty() {
        anyhow::bail!("terminal command の program が空です");
    }

    let bin_name = Path::new(&program)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    match bin_name {
        "xterm" => {
            args.push("-T".to_string());
            args.push(title.to_string());
            args.push("-e".to_string());
            args.push("bash".to_string());
            args.push("-lc".to_string());
            args.push(shell_command.to_string());
        }
        // Debian/Ubuntu alternatives entry. Many implementations (e.g. terminator)
        // reject "-- bash -lc ..." but accept "-e ...".
        "x-terminal-emulator" => {
            args.push("-T".to_string());
            args.push(title.to_string());
            args.push("-e".to_string());
            args.push("bash".to_string());
            args.push("-lc".to_string());
            args.push(shell_command.to_string());
        }
        "konsole" => {
            args.push("-p".to_string());
            args.push(format!("tabtitle={title}"));
            args.push("-e".to_string());
            args.push("bash".to_string());
            args.push("-lc".to_string());
            args.push(shell_command.to_string());
        }
        _ => {
            args.push("--title".to_string());
            args.push(title.to_string());
            args.push("--".to_string());
            args.push("bash".to_string());
            args.push("-lc".to_string());
            args.push(shell_command.to_string());
        }
    }

    Ok((program, args))
}

pub fn execute_action(action: ActionRequest) -> anyhow::Result<()> {
    match action {
        ActionRequest::OpenTarget(target) => {
            info!(target = %target, "open アクションを実行");
            let status = Command::new("xdg-open").arg(&target).status()?;
            if !status.success() {
                anyhow::bail!("xdg-open が失敗しました: status={status}");
            }
            Ok(())
        }
        ActionRequest::Command { program, args } => {
            if program.trim().is_empty() {
                anyhow::bail!("実行プログラム名が空です");
            }
            info!(program = %program, args = ?args, "command アクションを実行");
            let child = Command::new(&program).args(args).spawn()?;
            info!(program = %program, pid = child.id(), "command を非同期起動しました");
            Ok(())
        }
        ActionRequest::PodmanLogs {
            container_id,
            program,
            args,
        } => {
            if program.trim().is_empty() {
                anyhow::bail!("Podman logs 実行プログラム名が空です");
            }
            info!(
                container_id = %container_id,
                program = %program,
                args = ?args,
                "Podman logs アクションを実行"
            );
            let child = Command::new(&program).args(args).spawn()?;
            info!(
                container_id = %container_id,
                program = %program,
                pid = child.id(),
                "Podman logs を非同期起動しました"
            );
            Ok(())
        }
    }
}

pub fn execute_payload(payload: &str) -> anyhow::Result<()> {
    let trimmed = payload.trim();
    if trimmed.is_empty() {
        anyhow::bail!("action_payload が空です");
    }
    execute_action(ActionRequest::OpenTarget(trimmed.to_owned()))
}
