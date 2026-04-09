//! diagnose サブコマンド: Stream Deck 接続環境を段階的に診断する

use std::path::{Path, PathBuf};

use elgato_streamdeck::{list_devices, new_hidapi, StreamDeck};

/// Elgato の USB ベンダー ID
const ELGATO_VID_STR: &str = "0fd9";
/// Stream Deck+ の製品 ID
const PLUS_PID_STR: &str = "0084";

// ── ANSI カラーコード ─────────────────────────────────────────
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const RESET: &str = "\x1b[0m";

fn ok_mark() -> String {
    format!("{GREEN}✓ OK{RESET}")
}
fn ng_mark() -> String {
    format!("{RED}✗ NG{RESET}")
}
fn warn_mark() -> String {
    format!("{YELLOW}! WARN{RESET}")
}

/// 診断ステップの結果
enum StepResult {
    Ok,
    Warn,
    Ng,
}

/// 診断を実行して標準出力に結果を出力する
pub fn run() -> anyhow::Result<()> {
    println!("Stream Deck 接続診断");
    println!("{}", "═".repeat(54));
    println!();

    let mut passed = 0u32;
    let total = 8u32;

    // ── [1] lsusb コマンド ───────────────────────────────────
    match check_lsusb() {
        (StepResult::Ok, msg) => {
            println!("[1/{total}] lsusb … {}", ok_mark());
            for line in &msg {
                println!("      {line}");
            }
            passed += 1;
        }
        (StepResult::Warn, msg) => {
            println!(
                "[1/{total}] lsusb … {} (コマンドなし・スキップ)",
                warn_mark()
            );
            for line in &msg {
                println!("      {line}");
            }
            passed += 1;
        }
        (StepResult::Ng, msg) => {
            println!("[1/{total}] lsusb … {}", ng_mark());
            for line in &msg {
                println!("      {line}");
            }
            println!("      → USB に Elgato デバイス (VID={ELGATO_VID_STR}) が見えません。");
            println!("        ケーブルの接続またはデバイスの電源を確認してください。");
        }
    }
    println!();

    // ── [2] USB ドライババインド確認 ─────────────────────────
    let usb_path = find_elgato_usb_device();
    match check_usb_driver_bound(&usb_path) {
        (StepResult::Ok, msg) => {
            println!("[2/{total}] USB ドライババインド … {}", ok_mark());
            for line in &msg {
                println!("      {line}");
            }
            passed += 1;
        }
        (StepResult::Warn, msg) => {
            println!("[2/{total}] USB ドライババインド … {}", warn_mark());
            for line in &msg {
                println!("      {line}");
            }
        }
        (StepResult::Ng, msg) => {
            println!("[2/{total}] USB ドライババインド … {}", ng_mark());
            for line in &msg {
                println!("      {line}");
            }
            println!("      → カーネルの HID ドライバが Stream Deck+ にバインドされていません。");
            println!("        次のいずれかを実行してください:");
            println!("          1) デバイスを抜き挿しする (最も確実)");
            println!("          2) sudo udevadm trigger --attr-match=idVendor={ELGATO_VID_STR}");
            if let Some(iface) = find_elgato_interface_path() {
                println!(
                    "          3) echo \"{}\" | sudo tee /sys/bus/usb/drivers/usbhid/bind",
                    iface.display()
                );
            }
        }
    }
    println!();

    // ── [3] hidraw デバイスと権限 ────────────────────────────
    match check_hidraw() {
        (StepResult::Ok, msg) => {
            println!("[3/{total}] hidraw デバイス … {}", ok_mark());
            for line in &msg {
                println!("      {line}");
            }
            passed += 1;
        }
        (StepResult::Warn, msg) => {
            println!("[3/{total}] hidraw デバイス … {}", warn_mark());
            for line in &msg {
                println!("      {line}");
            }
            println!("      → デバイスは見つかりましたが権限が不足しています。");
            println!("        udev rules ([4]) と plugdev グループ ([5]) を確認してください。");
        }
        (StepResult::Ng, msg) => {
            println!("[3/{total}] hidraw デバイス … {}", ng_mark());
            for line in &msg {
                println!("      {line}");
            }
            println!("      → USB ドライバがバインドされると hidraw が作成されます。[2] を修正後に再診断。");
        }
    }
    println!();

    // ── [4] udev ルールファイル ──────────────────────────────
    match check_udev_rules() {
        (StepResult::Ok, msg) => {
            println!("[4/{total}] udev ルール … {}", ok_mark());
            for line in &msg {
                println!("      {line}");
            }
            passed += 1;
        }
        (_, msg) => {
            println!("[4/{total}] udev ルール … {}", ng_mark());
            for line in &msg {
                println!("      {line}");
            }
            println!("      → 修正コマンド:");
            println!(
                r#"        sudo tee /etc/udev/rules.d/50-streamdeck.rules << 'EOF'
SUBSYSTEM=="usb", ATTRS{{idVendor}}=="{ELGATO_VID_STR}", ATTRS{{idProduct}}=="{PLUS_PID_STR}", MODE="0666", GROUP="plugdev"
KERNEL=="hidraw*", ATTRS{{idVendor}}=="{ELGATO_VID_STR}", ATTRS{{idProduct}}=="{PLUS_PID_STR}", MODE="0666", GROUP="plugdev"
EOF
        sudo udevadm control --reload-rules && sudo udevadm trigger"#
            );
        }
    }
    println!();

    // ── [5] plugdev グループ ─────────────────────────────────
    match check_plugdev_group() {
        (StepResult::Ok, msg) => {
            println!("[5/{total}] plugdev グループ … {}", ok_mark());
            for line in &msg {
                println!("      {line}");
            }
            passed += 1;
        }
        (_, msg) => {
            println!("[5/{total}] plugdev グループ … {}", ng_mark());
            for line in &msg {
                println!("      {line}");
            }
            println!("      → 修正コマンド (ログアウト・再ログインが必要):");
            println!("        sudo usermod -aG plugdev $USER");
        }
    }
    println!();

    // ── [6] HidApi 初期化 ────────────────────────────────────
    match new_hidapi() {
        Ok(hid) => {
            println!("[6/{total}] HidApi 初期化 … {}", ok_mark());
            passed += 1;
            println!();

            // ── [7] デバイス列挙 ─────────────────────────────
            let devices = list_devices(&hid);
            if devices.is_empty() {
                println!("[7/{total}] デバイス列挙 … {}", ng_mark());
                println!("      Stream Deck デバイスが列挙されませんでした。");
                println!(
                    "      [2] USB ドライババインドを修正後、デバイスを抜き挿ししてください。"
                );
                println!();
                println!("[8/{total}] 接続テスト … スキップ");
            } else {
                println!(
                    "[7/{total}] デバイス列挙 … {} ({} 台)",
                    ok_mark(),
                    devices.len()
                );
                for (kind, serial) in &devices {
                    println!("      {kind:?} (シリアル: {serial})");
                }
                passed += 1;
                println!();

                // ── [8] 接続テスト ───────────────────────────
                let (kind, serial) = &devices[0];
                match StreamDeck::connect(&hid, *kind, serial) {
                    Ok(deck) => {
                        println!("[8/{total}] 接続テスト … {}", ok_mark());
                        let product = deck.product().unwrap_or_else(|_| "-".to_string());
                        let firmware = deck.firmware_version().unwrap_or_else(|_| "-".to_string());
                        println!("      製品名       : {product}");
                        println!("      シリアル     : {serial}");
                        println!("      ファームウェア: {firmware}");
                        if let Some((w, h)) = kind.lcd_strip_size() {
                            println!("      LCD サイズ   : {w}x{h} px");
                        }
                        passed += 1;
                    }
                    Err(e) => {
                        println!("[8/{total}] 接続テスト … {}", ng_mark());
                        println!("      エラー: {e}");
                        println!("      → 別のプロセスがデバイスを占有している可能性があります。");
                    }
                }
            }
        }
        Err(e) => {
            println!("[6/{total}] HidApi 初期化 … {}", ng_mark());
            println!("      エラー: {e}");
            println!("      → libhidapi-dev がインストールされているか確認してください:");
            println!("        sudo apt install libhidapi-dev");
            println!();
            println!("[7/{total}] デバイス列挙 … スキップ");
            println!("[8/{total}] 接続テスト … スキップ");
        }
    }

    // ── サマリ ────────────────────────────────────────────────
    println!();
    println!("{}", "─".repeat(54));
    if passed == total {
        println!("診断結果: {}/{} チェック正常 {}", passed, total, ok_mark());
    } else {
        println!("診断結果: {}/{} チェック正常 {}", passed, total, ng_mark());
        println!("上記の NG 項目を修正し、再度 `stream-deck diagnose` を実行してください。");
    }

    Ok(())
}

// ── 各チェック実装 ────────────────────────────────────────────

/// lsusb コマンドで Elgato USB デバイスを確認する
fn check_lsusb() -> (StepResult, Vec<String>) {
    match std::process::Command::new("lsusb").output() {
        Err(_) => (
            StepResult::Warn,
            vec!["lsusb コマンドが見つかりません (usbutils 未インストール)".to_string()],
        ),
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let matches: Vec<String> = stdout
                .lines()
                .filter(|l| l.to_lowercase().contains(ELGATO_VID_STR))
                .map(|l| l.to_string())
                .collect();
            if matches.is_empty() {
                (
                    StepResult::Ng,
                    vec!["Elgato デバイスが lsusb に見えません。".to_string()],
                )
            } else {
                (StepResult::Ok, matches)
            }
        }
    }
}

/// /sys/bus/usb/devices/ から Elgato の USB デバイスパスを探す
fn find_elgato_usb_device() -> Option<PathBuf> {
    let Ok(entries) = std::fs::read_dir("/sys/bus/usb/devices") else {
        return None;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let vid_path = path.join("idVendor");
        let pid_path = path.join("idProduct");
        let vid = std::fs::read_to_string(&vid_path)
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        let pid = std::fs::read_to_string(&pid_path)
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        if vid == ELGATO_VID_STR && pid == PLUS_PID_STR {
            return Some(path);
        }
    }
    None
}

/// Elgato デバイスの USB インターフェースパス (例: "1-1.2:1.0") を探す
fn find_elgato_interface_path() -> Option<PathBuf> {
    let dev_path = find_elgato_usb_device()?;
    let dev_name = dev_path.file_name()?.to_string_lossy().to_string();
    // インターフェースは "<busport>:N.M" の形式
    let iface_name = format!("{dev_name}:1.0");
    let iface_path = Path::new("/sys/bus/usb/devices").join(&iface_name);
    if iface_path.exists() {
        Some(PathBuf::from(iface_name))
    } else {
        None
    }
}

/// USB インターフェースにカーネルドライバがバインドされているか確認する
fn check_usb_driver_bound(usb_path: &Option<PathBuf>) -> (StepResult, Vec<String>) {
    let Some(dev_path) = usb_path else {
        return (
            StepResult::Ng,
            vec![
                format!(
                    "/sys/bus/usb/devices/ に Elgato デバイス (VID={ELGATO_VID_STR}) が見当たりません。"
                ),
            ],
        );
    };

    let dev_name = dev_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    // インターフェース "busport:config.iface" を探す
    let Ok(entries) = std::fs::read_dir("/sys/bus/usb/devices") else {
        return (
            StepResult::Ng,
            vec!["sysfs の読み取りに失敗しました".to_string()],
        );
    };

    let mut bound: Vec<String> = Vec::new();
    let mut unbound: Vec<String> = Vec::new();

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        // インターフェースは "<dev_name>:N.M" の形式
        if !name.starts_with(&format!("{dev_name}:")) {
            continue;
        }
        let driver_path = entry.path().join("driver");
        let iface_class_path = entry.path().join("bInterfaceClass");
        let class = std::fs::read_to_string(iface_class_path)
            .unwrap_or_default()
            .trim()
            .to_string();

        if driver_path.exists() {
            // driver シンボリックリンクの先のドライバ名を取得
            let driver_name = std::fs::read_link(&driver_path)
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                .unwrap_or_else(|| "unknown".to_string());
            bound.push(format!(
                "{name} (class=0x{class}) → ドライバ: {driver_name}"
            ));
        } else {
            unbound.push(format!(
                "{name} (class=0x{class}) → {RED}ドライバ未バインド{RESET}"
            ));
        }
    }

    if unbound.is_empty() && !bound.is_empty() {
        (StepResult::Ok, bound)
    } else if !unbound.is_empty() {
        let mut msgs = bound;
        msgs.extend(unbound);
        (StepResult::Ng, msgs)
    } else {
        (
            StepResult::Warn,
            vec![format!(
                "デバイス {dev_name} のインターフェースが見つかりませんでした。"
            )],
        )
    }
}

/// /sys/class/hidraw を走査して Elgato デバイスの hidraw エントリと権限を確認する
fn check_hidraw() -> (StepResult, Vec<String>) {
    let hidraw_dir = Path::new("/sys/class/hidraw");
    if !hidraw_dir.exists() {
        return (
            StepResult::Ng,
            vec!["/sys/class/hidraw が存在しません (カーネル設定を確認)".to_string()],
        );
    }

    let mut found: Vec<(String, bool)> = Vec::new();

    let Ok(entries) = std::fs::read_dir(hidraw_dir) else {
        return (
            StepResult::Ng,
            vec!["/sys/class/hidraw の読み取りに失敗".to_string()],
        );
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let uevent_path = format!("/sys/class/hidraw/{name}/device/uevent");
        let Ok(uevent) = std::fs::read_to_string(&uevent_path) else {
            continue;
        };
        // HID_ID=0003:00000FD9:00000084 形式でマッチ
        if !uevent
            .to_uppercase()
            .contains(&ELGATO_VID_STR.to_uppercase())
        {
            continue;
        }
        let dev_path = format!("/dev/{name}");
        let accessible = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&dev_path)
            .is_ok();
        found.push((dev_path, accessible));
    }

    if found.is_empty() {
        return (
            StepResult::Ng,
            vec!["Elgato に対応する hidraw エントリがありません。".to_string()],
        );
    }

    let all_ok = found.iter().all(|(_, ok)| *ok);
    let msgs: Vec<String> = found
        .iter()
        .map(|(path, ok)| {
            if *ok {
                format!("{path} … 読み書き可")
            } else {
                format!("{path} … {RED}権限なし{RESET} (udev rules を確認)")
            }
        })
        .collect();

    if all_ok {
        (StepResult::Ok, msgs)
    } else {
        (StepResult::Warn, msgs)
    }
}

/// udev ルールファイルの存在を確認する
fn check_udev_rules() -> (StepResult, Vec<String>) {
    let candidates = [
        "/etc/udev/rules.d/50-streamdeck.rules",
        "/etc/udev/rules.d/99-streamdeck.rules",
        "/lib/udev/rules.d/50-streamdeck.rules",
    ];

    let found: Vec<&str> = candidates
        .iter()
        .copied()
        .filter(|p| Path::new(p).exists())
        .collect();

    if found.is_empty() {
        (
            StepResult::Ng,
            vec!["Stream Deck 用 udev ルールファイルが見つかりません。".to_string()],
        )
    } else {
        (
            StepResult::Ok,
            found.iter().map(|s| s.to_string()).collect(),
        )
    }
}

/// カレントユーザーが plugdev グループに所属しているか確認する
fn check_plugdev_group() -> (StepResult, Vec<String>) {
    match std::process::Command::new("id").arg("-Gn").output() {
        Err(e) => (StepResult::Ng, vec![format!("id コマンド失敗: {e}")]),
        Ok(out) => {
            let groups = String::from_utf8_lossy(&out.stdout);
            let username = std::process::Command::new("id")
                .arg("-un")
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_else(|_| "?".to_string());

            if groups.split_whitespace().any(|g| g == "plugdev") {
                (
                    StepResult::Ok,
                    vec![format!(
                        "ユーザー \"{username}\" は plugdev グループに所属しています"
                    )],
                )
            } else {
                (
                    StepResult::Ng,
                    vec![
                        format!("ユーザー \"{username}\" は plugdev グループに所属していません"),
                        format!("現在のグループ: {}", groups.trim()),
                    ],
                )
            }
        }
    }
}
