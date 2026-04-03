# stream-deck

Elgato Stream Deck+ 向けの操作ランチャーです。LCD にシステムメトリクスを表示しながら、ボタンでアプリ起動・SSH 接続・ページ遷移を操作できます。

## 機能

- **アプリランチャー**: 設定ファイルで定義したコマンドをボタン一発で実行する
- **SSH ナビゲーション**: SSH ホスト一覧からページ選択して接続し、既存セッションへ再フォーカスする
- **LCD メトリクス表示**: CPU / メモリ / ロードを常時表示する
- **通知表示**: 外部通知をボタンへオーバーレイ表示する
- **自動コンテキスト遷移**: アクティブウィンドウから SSH ホストを検出して対応ページへ自動移動する

## セットアップ

### 必須パッケージ

```bash
# udev rules（Stream Deck を plugdev グループで使うため）
sudo tee /etc/udev/rules.d/50-streamdeck.rules <<'EOF'
SUBSYSTEM=="usb", ATTRS{idVendor}=="0fd9", ATTRS{idProduct}=="0084", GROUP="plugdev", MODE="0664"
EOF
sudo udevadm control --reload-rules
sudo udevadm trigger
# デバイスを抜き挿しして認識させる

# wmctrl / xdotool（SSH ウィンドウフォーカス用）
sudo apt install wmctrl xdotool

# Python DBus ヘルパーの依存（window_tracker.py 用）
sudo apt install python3-gi python3-dbus
```

> **注意**: udev rules を設定しても usbhid がバインドされたままの場合はデバイスを抜き挿しすると解消します。

### ビルドと起動

```bash
cargo build -p stream-deck

RUST_LOG=info cargo run -p stream-deck -- \
  --config ./crates/stream-deck/config/layout.toml
```

診断コマンド（デバイス認識から接続まで 8 段階で確認）:

```bash
cargo run -p stream-deck -- diagnose
```

## 設定ファイル

`layout.toml` はページ中心の構成で記述します。

```toml
[app]
home = "home"

[[pages]]
id = "home"
title = "Home"

[[pages.items]]
kind = "nav"
label = "Apps"
target = "apps"

[[pages.items]]
kind = "nav"
label = "SSH"
target = "ssh_hosts"

[[pages]]
id = "apps"
title = "Apps"

[[pages.items]]
kind = "back"
label = "Back"
priority = 10

[[pages.items]]
kind = "command"
label = "Browser"
command = ["xdg-open", "https://example.com"]

[dynamic.ssh_hosts]
source = "crates/stream-deck/config/ssh_hosts.toml"
page_size = 6
page_id_prefix = "ssh_hosts"
terminal = ["/usr/bin/x-terminal-emulator"]
ssh_template = "ssh -o StrictHostKeyChecking=accept-new {host}"
```

`ssh_hosts.toml`:

```toml
[[hosts]]
id = "server-01"
label = "Server01"
host = "user@192.168.0.11"
```

### priority の使い方

`priority` が低いほど先のスロットへ割り当てられます。推奨レンジ:

| 用途 | priority |
|------|---------|
| Back / 戻る系 | 10 |
| Prev / Next（ページング） | 20 |
| 通常アプリ・SSH ホスト | 50〜100（省略可） |

## SSH ナビゲーション

### ウィンドウフォーカス（Wayland 環境）

Wayland セッションでは `wmctrl`/`xdotool` はデフォルトで Wayland ネイティブアプリに届きません。端末エミュレータを **XWayland** 経由で起動することで既存セッションへのフォーカスが機能します。

```toml
[dynamic.ssh_hosts]
# GDK_BACKEND=x11 を先頭に追加して XWayland で起動する
terminal = ["env", "GDK_BACKEND=x11", "/usr/bin/x-terminal-emulator"]
```

X11 セッションの場合はそのまま `/usr/bin/x-terminal-emulator` で動作します。

### セッション再利用の仕組み

- 同一ホストへの 2 回目以降の押下は PID 生存チェックを経て既存セッションへフォーカスする
- PID が死んでいた場合は自動で新規起動する
- フォーカスは `wmctrl -a <title>` → `xdotool search --name <title> windowactivate` の順で試みる

## 自動コンテキスト遷移（DBus ヘルパー）

SSH 端末をフォアグラウンドにしたとき、Stream Deck のページが自動で対応 SSH ページへ切り替わります。

### `window_tracker.py` の起動

AT-SPI でアクティブウィンドウタイトルを取得し、DBus プロパティとして公開するヘルパーです。

```bash
# /usr/bin/python3 を明示する（python3-gi が入っているインタープリタ）
# -u でバッファリングを無効化してログを即時出力する
/usr/bin/python3 -u crates/stream-deck/window_tracker.py > /tmp/tracker.log 2>&1 &

# 動作確認
gdbus call --session \
  --dest io.github.uzuna.StreamDeckHelper \
  --object-path /io/github/uzuna/StreamDeckHelper \
  --method org.freedesktop.DBus.Properties.Get \
  io.github.uzuna.StreamDeckHelper ActiveWindowTitle
```

### stream-deck 側の環境変数

| 変数 | 値 |
|------|----|
| `STREAM_DECK_DBUS_SERVICE` | `io.github.uzuna.StreamDeckHelper` |
| `STREAM_DECK_DBUS_PATH` | `/io/github/uzuna/StreamDeckHelper` |
| `STREAM_DECK_DBUS_INTERFACE` | `io.github.uzuna.StreamDeckHelper` |
| `STREAM_DECK_DBUS_PROPERTY` | `ActiveWindowTitle` |

```bash
export STREAM_DECK_DBUS_SERVICE="io.github.uzuna.StreamDeckHelper"
export STREAM_DECK_DBUS_PATH="/io/github/uzuna/StreamDeckHelper"
export STREAM_DECK_DBUS_INTERFACE="io.github.uzuna.StreamDeckHelper"
export STREAM_DECK_DBUS_PROPERTY="ActiveWindowTitle"

RUST_LOG=info cargo run -p stream-deck -- \
  --config ./crates/stream-deck/config/layout.toml
```

環境変数が未設定の場合は no-op で継続します（自動遷移なし）。

### 仕組み

- `window_tracker.py` は 500ms ごとに AT-SPI でアクティブウィンドウタイトルを取得して DBus プロパティに書き込む
- stream-deck は 500ms ごとに DBus プロパティを読み、タイトルに `rs-common:ssh:<host>` が含まれていれば対応 SSH ページへ自動遷移する

## SSH テスト環境（multipass）

```bash
# VM を作成して設定ファイルを自動生成する
bash scripts/stream-deck/setup_multipass_ssh_test.sh

# 起動
RUST_LOG=info cargo run -p stream-deck -- \
  --config ./crates/stream-deck/config/layout.ssh-test.toml

# 後片付け
bash scripts/stream-deck/cleanup_multipass_ssh_test.sh
```

| 環境変数 | デフォルト | 説明 |
|----------|-----------|------|
| `VM_NAME` | `streamdeck-ssh-test` | VM 名 |
| `HOST_COUNT` | `1` | 生成するホスト数（ページングテストは `9` 以上） |
| `SSH_USER` | `ubuntu` | SSH ユーザー名 |
| `SSH_PAGE_SIZE` | `6` | 1 ページあたりのホスト数 |
