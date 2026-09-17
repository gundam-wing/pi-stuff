#!/usr/bin/env bash
# Run on the Pi as root (via sudo). Starts nixos-rebuild as a oneshot systemd
# unit so a long kernel/nixpkgs rebuild can continue overnight after SSH drops.
#
# Progress is written to REBUILD_LOG with --print-build-logs, plus a heartbeat
# every HEARTBEAT_SECS that records load and active build processes. Use
# scripts/rebuild-status.sh to inspect without attaching.
set -euo pipefail

flake_attr="${FLAKE_ATTR:-myhostname}"
jobs="${PI_JOBS:-2}"
cores="${PI_CORES:-2}"
remote_dir="${PI_DIR:-$(cd "$(dirname "$0")/.." && pwd)}"
rebuild_log="${REBUILD_LOG:-/var/log/nixos-rebuild-overnight.log}"
unit="${SYSTEMD_UNIT:-nixos-rebuild-overnight.service}"
heartbeat_secs="${HEARTBEAT_SECS:-120}"
runner="${RUNNER_PATH:-/var/lib/nixos-rebuild-overnight-runner.sh}"

if [[ "$(id -u)" -ne 0 ]]; then
  echo "run as root: sudo $0" >&2
  exit 1
fi

state="$(systemctl is-active "$unit" 2>/dev/null || true)"
substate="$(systemctl show -p SubState --value "$unit" 2>/dev/null || true)"
case "$state" in
  activating|reloading)
    echo "unit $unit is already $state ($substate)"
    echo "Status: sudo $remote_dir/scripts/rebuild-status.sh"
    echo "Log:    sudo tail -f $rebuild_log"
    exit 1
    ;;
  active)
    if [[ "$substate" == "exited" ]]; then
      # Prior successful RemainAfterExit unit; clear it so a new run can start.
      systemctl stop "$unit" 2>/dev/null || true
    else
      echo "unit $unit is already $state ($substate)"
      echo "Status: sudo $remote_dir/scripts/rebuild-status.sh"
      echo "Log:    sudo tail -f $rebuild_log"
      exit 1
    fi
    ;;
esac

systemctl reset-failed "$unit" 2>/dev/null || true
: > "$rebuild_log"
chmod 0644 "$rebuild_log"

# Wrapper keeps a heartbeat in the same log so a quiet kernel compile still
# shows the job is alive. -L prints each build's stdout as it runs.
cat >"$runner" <<EOF
#!/usr/bin/env bash
set -uo pipefail
log=$(printf '%q' "$rebuild_log")
hb=$(printf '%q' "$heartbeat_secs")
flake=$(printf '%q' "$remote_dir#$flake_attr")
jobs=$(printf '%q' "$jobs")
cores=$(printf '%q' "$cores")

heartbeat() {
  while true; do
    sleep "\$hb"
    {
      echo ""
      echo "==== heartbeat \$(date -Is) load=\$(cut -d' ' -f1-3 /proc/loadavg) ===="
      ps -eo pid,etime,pcpu,pmem,cmd --sort=-pcpu \\
        | grep -E 'nix-build|cc1|cc1plus|rustc|cargo|ld |cmake|ninja' \\
        | grep -v grep \\
        | head -n 8 \\
        || echo "(no compiler processes this interval)"
      if compgen -G '/tmp/nix-build-*' >/dev/null; then
        ls -ldt /tmp/nix-build-* 2>/dev/null | head -n 5
      fi
    } >>"\$log"
  done
}

echo "==== overnight rebuild start \$(date -Is) flake=\$flake jobs=\$jobs cores=\$cores ====" >>"\$log"
heartbeat &
hb_pid=\$!
trap 'kill \$hb_pid 2>/dev/null || true' EXIT

if command -v stdbuf >/dev/null; then
  stdbuf -oL -eL nixos-rebuild switch -L \\
    --max-jobs "\$jobs" \\
    --cores "\$cores" \\
    --flake "\$flake" >>"\$log" 2>&1
  rc=\$?
else
  nixos-rebuild switch -L \\
    --max-jobs "\$jobs" \\
    --cores "\$cores" \\
    --flake "\$flake" >>"\$log" 2>&1
  rc=\$?
fi

echo "==== overnight rebuild end \$(date -Is) exit=\$rc ====" >>"\$log"
exit "\$rc"
EOF
chmod 0700 "$runner"

# Keep the unit around after exit so status/result remain inspectable.
systemd-run \
  --no-block \
  --unit="${unit%.service}" \
  --working-directory="$remote_dir" \
  --property=Type=oneshot \
  --property=RemainAfterExit=yes \
  /bin/bash "$runner"

echo "Overnight rebuild started as $unit"
echo "Progress: sudo $remote_dir/scripts/rebuild-status.sh"
echo "Follow:   sudo $remote_dir/scripts/rebuild-status.sh --follow"
echo "Log:      sudo tail -f $rebuild_log"
echo "Success ends in ActiveState=active / Result=success; failure is Result=failed."
