#!/usr/bin/env bash
set -euo pipefail

VM_NAME="${VM_NAME:-streamdeck-ssh-test}"
VM_IMAGE="${VM_IMAGE:-24.04}"
VM_CPUS="${VM_CPUS:-1}"
VM_MEM="${VM_MEM:-1G}"
VM_DISK="${VM_DISK:-8G}"
HOST_COUNT="${HOST_COUNT:-1}"
SSH_USER="${SSH_USER:-ubuntu}"
SSH_PAGE_SIZE="${SSH_PAGE_SIZE:-6}"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CONFIG_DIR="$ROOT_DIR/crates/stream-deck/config"
HOSTS_FILE="$CONFIG_DIR/ssh_hosts.toml"
LAYOUT_FILE="$CONFIG_DIR/layout.ssh-test.toml"

require_cmd() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "ERROR: required command not found: $cmd" >&2
    exit 1
  fi
}

pick_or_create_ssh_pubkey() {
  local pubkey
  for pubkey in "$HOME/.ssh/id_ed25519.pub" "$HOME/.ssh/id_rsa.pub"; do
    if [[ -f "$pubkey" ]]; then
      echo "$pubkey"
      return
    fi
  done

  local key_file="$HOME/.ssh/id_ed25519"
  echo "No SSH key found. Generating test key: $key_file"
  mkdir -p "$HOME/.ssh"
  chmod 700 "$HOME/.ssh"
  ssh-keygen -t ed25519 -N "" -f "$key_file" >/dev/null
  echo "$key_file.pub"
}

wait_vm_ip() {
  local vm_name="$1"
  local tries=30
  local ip=""

  for ((i=1; i<=tries; i++)); do
    ip="$(multipass info "$vm_name" | awk '/IPv4/{print $2; exit}')"
    if [[ -n "$ip" && "$ip" != "--" ]]; then
      echo "$ip"
      return
    fi
    sleep 1
  done

  echo "ERROR: failed to get IPv4 address for VM: $vm_name" >&2
  exit 1
}

render_ssh_hosts_file() {
  local host_count="$1"
  local ssh_user="$2"
  local vm_ip="$3"

  {
    for i in $(seq 1 "$host_count"); do
      printf '[[hosts]]\n'
      printf 'id = "vm-%02d"\n' "$i"
      printf 'label = "VM%02d"\n' "$i"
      printf 'host = "%s@%s"\n' "$ssh_user" "$vm_ip"
      printf 'tags = ["multipass", "ssh-test"]\n\n'
    done
  } >"$HOSTS_FILE"
}

render_layout_file() {
  cat >"$LAYOUT_FILE" <<'EOF'
# Stream Deck レイアウト設定 (SSH test 用)

[app]
home = "home"

[dashboard]

[[dashboard.sections]]
type = "cpu"
capacity = 10

[[dashboard.sections]]
type = "mem"
capacity = 30

[[dashboard.sections]]
type = "load"
capacity = 30

[[dashboard.sections]]
type = "notif"
capacity = 12

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
priority = -100

[[pages.items]]
kind = "command"
label = "Terminal"
command = ["/usr/bin/x-terminal-emulator"]
priority = 0

[dynamic.ssh_hosts]
source = "ssh_hosts.toml"
page_size = __SSH_PAGE_SIZE__
page_id_prefix = "ssh_hosts"
terminal = ["/usr/bin/x-terminal-emulator"]
ssh_template = "ssh -o StrictHostKeyChecking=accept-new {host}"
EOF

  # shellcheck disable=SC2016
  sed -i "s/__SSH_PAGE_SIZE__/$SSH_PAGE_SIZE/" "$LAYOUT_FILE"
}

main() {
  require_cmd multipass
  require_cmd ssh
  require_cmd ssh-keygen
  require_cmd awk
  require_cmd base64

  mkdir -p "$CONFIG_DIR"

  local pubkey
  pubkey="$(pick_or_create_ssh_pubkey)"
  if [[ ! -r "$pubkey" ]]; then
    echo "ERROR: public key is not readable: $pubkey" >&2
    exit 1
  fi

  if multipass info "$VM_NAME" >/dev/null 2>&1; then
    echo "VM already exists: $VM_NAME (starting if needed)"
    multipass start "$VM_NAME" >/dev/null
  else
    echo "Launching VM: $VM_NAME"
    multipass launch "$VM_IMAGE" \
      --name "$VM_NAME" \
      --cpus "$VM_CPUS" \
      --memory "$VM_MEM" \
      --disk "$VM_DISK" >/dev/null
  fi

  local vm_ip
  vm_ip="$(wait_vm_ip "$VM_NAME")"

  echo "Installing host public key into VM ($pubkey)"
  local pubkey_b64
  pubkey_b64="$(base64 -w0 < "$pubkey")"
  multipass exec "$VM_NAME" -- bash -lc 'mkdir -p ~/.ssh && chmod 700 ~/.ssh && touch ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys'
  multipass exec "$VM_NAME" -- bash -lc "printf '%s' '$pubkey_b64' | base64 -d >> ~/.ssh/authorized_keys && sort -u ~/.ssh/authorized_keys -o ~/.ssh/authorized_keys"

  echo "Checking SSH connectivity"
  ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new -o ConnectTimeout=5 \
    "$SSH_USER@$vm_ip" "echo ssh-ok" >/dev/null

  render_ssh_hosts_file "$HOST_COUNT" "$SSH_USER" "$vm_ip"
  render_layout_file

  cat <<MSG
Setup completed.

VM Name:       $VM_NAME
VM Address:    $vm_ip
SSH User:      $SSH_USER
Hosts file:    $HOSTS_FILE
Layout file:   $LAYOUT_FILE

Run stream-deck with:
  cargo run -p stream-deck -- --config "$LAYOUT_FILE"

To generate paging test data (more than 8 hosts):
  HOST_COUNT=9 scripts/stream-deck/setup_multipass_ssh_test.sh
MSG
}

main "$@"
