#!/usr/bin/env bash
set -euo pipefail

VM_NAME="${VM_NAME:-streamdeck-ssh-test}"
REMOVE_FILES="${REMOVE_FILES:-0}"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CONFIG_DIR="$ROOT_DIR/crates/stream-deck/config"
HOSTS_FILE="$CONFIG_DIR/ssh_hosts.toml"
LAYOUT_FILE="$CONFIG_DIR/layout.ssh-test.toml"

if command -v multipass >/dev/null 2>&1; then
  if multipass info "$VM_NAME" >/dev/null 2>&1; then
    echo "Deleting VM: $VM_NAME"
    multipass delete "$VM_NAME" >/dev/null
    multipass purge >/dev/null
  else
    echo "VM not found: $VM_NAME"
  fi
else
  echo "multipass command not found; skipping VM cleanup"
fi

if [[ "$REMOVE_FILES" == "1" ]]; then
  rm -f "$HOSTS_FILE" "$LAYOUT_FILE"
  echo "Removed generated config files"
fi

echo "Cleanup completed"
