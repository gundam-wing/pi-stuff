#!/usr/bin/env bash
# Snapshot progress of an overnight (or any) nixos-rebuild on this Pi.
# Safe to run while the rebuild is in progress; does not change anything.
set -euo pipefail

rebuild_log="${REBUILD_LOG:-/var/log/nixos-rebuild-overnight.log}"
unit="${SYSTEMD_UNIT:-nixos-rebuild-overnight.service}"
follow=0

case "${1:-}" in
  "") ;;
  -f|--follow) follow=1 ;;
  -h|--help)
    echo "usage: $0 [-f|--follow]"
    echo "  Print a rebuild progress snapshot (unit state, log tail, active builds)."
    echo "  --follow refreshes every 30s until Ctrl-C."
    exit 0
    ;;
  *)
    echo "usage: $0 [-f|--follow]" >&2
    exit 2
    ;;
esac

print_status() {
  local now
  now="$(date -Is)"
  echo "=== rebuild status @ $now ==="

  if systemctl cat "$unit" &>/dev/null; then
    echo "-- unit: $unit"
    systemctl show "$unit" -p ActiveState -p SubState -p Result -p ExecMainStartTimestamp -p ExecMainStatus --no-pager 2>/dev/null || true
    systemctl --no-pager --full status "$unit" 2>/dev/null | head -n 20 || true
  else
    echo "-- unit: $unit (not loaded — finished with --collect, never started, or different name)"
  fi

  echo
  echo "-- log: $rebuild_log"
  if [[ -f "$rebuild_log" ]]; then
    local lines bytes mtime
    lines="$(wc -l <"$rebuild_log" | tr -d ' ')"
    bytes="$(wc -c <"$rebuild_log" | tr -d ' ')"
    mtime="$(date -Is -r "$rebuild_log" 2>/dev/null || stat -c %y "$rebuild_log" 2>/dev/null || echo unknown)"
    echo "size=${bytes}B lines=${lines} mtime=${mtime}"
    if [[ "$bytes" -eq 0 ]]; then
      echo "(empty — rebuild may be evaluating, or output is only in the journal)"
    else
      echo "---- last 25 log lines ----"
      tail -n 25 "$rebuild_log"
      echo "---- end ----"
    fi
  else
    echo "(missing)"
  fi

  echo
  echo "-- nix / compiler activity"
  # shellcheck disable=SC2009
  ps -eo pid,etime,pcpu,pmem,stat,cmd --sort=-pcpu \
    | grep -E 'nixos-rebuild|nix-daemon|nix-build|cc1|cc1plus|rustc|cargo|ld |cmake|ninja|npm |node ' \
    | grep -v grep \
    | head -n 20 \
    || echo "(no matching build processes — idle, stuck, or finished)"

  echo
  echo "-- active nix build dirs"
  # Prefer listing newest build scratch dirs; empty means download/eval or done.
  if compgen -G '/tmp/nix-build-*' >/dev/null; then
    ls -ldt /tmp/nix-build-* 2>/dev/null | head -n 10
  else
    echo "(none under /tmp/nix-build-*)"
  fi

  if [[ -d /nix/var/nix/builds ]]; then
    echo
    echo "-- /nix/var/nix/builds"
    ls -lt /nix/var/nix/builds 2>/dev/null | head -n 10 || true
  fi

  echo
  echo "Tips:"
  echo "  live log:    sudo tail -f $rebuild_log"
  echo "  live journal: sudo journalctl -u $unit -f"
  echo "  nix-daemon:  sudo journalctl -u nix-daemon -f"
}

if [[ "$(id -u)" -ne 0 ]]; then
  # Re-exec under sudo so journal/log paths are readable without juggling.
  exec sudo env REBUILD_LOG="$rebuild_log" SYSTEMD_UNIT="$unit" "$0" ${1:+"$1"}
fi

if [[ "$follow" -eq 1 ]]; then
  while true; do
    clear 2>/dev/null || true
    print_status
    sleep 30
  done
else
  print_status
fi
