#!/usr/bin/env bash
# Run on the Pi as root (via sudo). Starts nixos-rebuild as a oneshot systemd
# unit so a long kernel/nixpkgs rebuild can continue overnight after SSH drops.
set -euo pipefail

flake_attr="${FLAKE_ATTR:-myhostname}"
jobs="${PI_JOBS:-2}"
cores="${PI_CORES:-2}"
remote_dir="${PI_DIR:-$(cd "$(dirname "$0")/.." && pwd)}"
rebuild_log="${REBUILD_LOG:-/var/log/nixos-rebuild-overnight.log}"
unit="${SYSTEMD_UNIT:-nixos-rebuild-overnight.service}"

if [[ "$(id -u)" -ne 0 ]]; then
  echo "run as root: sudo $0" >&2
  exit 1
fi

state="$(systemctl is-active "$unit" 2>/dev/null || true)"
case "$state" in
  activating|active|reloading)
    echo "unit $unit is already $state"
    echo "Status: sudo systemctl status $unit"
    echo "Log:    sudo journalctl -u $unit -f"
    echo "File:   sudo tail -f $rebuild_log"
    exit 1
    ;;
esac

systemctl reset-failed "$unit" 2>/dev/null || true
: > "$rebuild_log"
chmod 0644 "$rebuild_log"

systemd-run \
  --no-block \
  --unit="${unit%.service}" \
  --collect \
  --working-directory="$remote_dir" \
  --property=Type=oneshot \
  --property=RemainAfterExit=yes \
  --property="StandardOutput=append:$rebuild_log" \
  --property="StandardError=append:$rebuild_log" \
  nixos-rebuild switch \
  --max-jobs "$jobs" \
  --cores "$cores" \
  --flake "$remote_dir#$flake_attr"

echo "Overnight rebuild started as $unit"
echo "Status: sudo systemctl status $unit"
echo "Log:    sudo journalctl -u $unit -f"
echo "File:   sudo tail -f $rebuild_log"
echo "Success ends in 'active (exited)'; failure is 'failed'."
