#!/usr/bin/env bash
set -euo pipefail

# Copy this flake to the Pi and switch the running NixOS generation.
# The monitor/ and web/ trees are flake inputs, so they must be on the Pi.
#
# Usage:
#   ./scripts/deploy.sh                # rsync, then nixos-rebuild switch
#   ./scripts/deploy.sh --sync-only    # rsync only
#   ./scripts/deploy.sh --overnight    # rsync, then start rebuild via systemd-run
#
# Overnight mode is for deliberate nixpkgs/kernel bumps that can run for hours
# on the capacity-constrained Pi. It prompts once for sudo, then continues as a
# oneshot systemd unit so the SSH client can disconnect safely.
#
# Optional environment:
#   PI_HOST       SSH target (default: guest@10.0.1.200)
#   PI_DIR        Remote flake path (default: /home/guest/pi-stuff)
#   FLAKE_ATTR    nixosConfigurations attr (default: myhostname)
#   PI_JOBS       nix max-jobs (default: 2)
#   PI_CORES      nix cores (default: 2)
#   REBUILD_LOG   remote log path (default: /var/log/nixos-rebuild-overnight.log)
#   SYSTEMD_UNIT  remote unit name (default: nixos-rebuild-overnight.service)

root="$(cd "$(dirname "$0")/.." && pwd)"
host="${PI_HOST:-guest@10.0.1.200}"
remote_dir="${PI_DIR:-/home/guest/pi-stuff}"
flake_attr="${FLAKE_ATTR:-myhostname}"
jobs="${PI_JOBS:-2}"
cores="${PI_CORES:-2}"
rebuild_log="${REBUILD_LOG:-/var/log/nixos-rebuild-overnight.log}"
systemd_unit="${SYSTEMD_UNIT:-nixos-rebuild-overnight.service}"
mode=switch

case "${1:-}" in
  "") ;;
  --sync-only) mode=sync-only ;;
  --overnight) mode=overnight ;;
  -h|--help)
    sed -n '2,24p' "$0" | sed 's/^# \{0,1\}//'
    exit 0
    ;;
  *)
    echo "usage: $0 [--sync-only|--overnight]" >&2
    exit 2
    ;;
esac

cd "$root"

# The Pi copy excludes .git, so bake a source revision into the tree that Nix
# can read at eval time. Prefer a dirty-aware git short hash over a store hash.
if rev="$(git rev-parse --short HEAD 2>/dev/null)"; then
  if [[ -n "$(git status --porcelain)" ]]; then
    rev="${rev}-dirty"
  fi
  printf '%s\n' "$rev" > "$root/REVISION"
  echo "Stamped revision $rev"
else
  echo "warning: git revision unavailable; Nix will fall back to flake metadata" >&2
fi

echo "Syncing $root -> $host:$remote_dir"
rsync -az --delete --delete-excluded \
  --exclude='.git/' \
  --exclude='.DS_Store' \
  --exclude='._*' \
  --exclude='images/' \
  --exclude='monitor/target/' \
  --exclude='web/.direnv/' \
  --exclude='web/dist/' \
  --exclude='web/node_modules/' \
  --exclude='result' \
  --exclude='result-*' \
  --exclude='*.m3u8' \
  --exclude='segment-*.ts' \
  ./ "$host:$remote_dir/"

rebuild_cmd="nixos-rebuild switch --max-jobs $jobs --cores $cores --flake $remote_dir#$flake_attr"

if [[ "$mode" == "sync-only" ]]; then
  echo "Copied. Rebuild later with:"
  echo "  ssh -t $host 'sudo $rebuild_cmd'"
  echo "Or start an overnight detached rebuild with:"
  echo "  $0 --overnight"
  exit 0
fi

if [[ "$mode" == "overnight" ]]; then
  echo "Starting overnight rebuild on $host ($systemd_unit)"
  ssh -t "$host" \
    "sudo env \
      PI_DIR=$(printf '%q' "$remote_dir") \
      FLAKE_ATTR=$(printf '%q' "$flake_attr") \
      PI_JOBS=$(printf '%q' "$jobs") \
      PI_CORES=$(printf '%q' "$cores") \
      REBUILD_LOG=$(printf '%q' "$rebuild_log") \
      SYSTEMD_UNIT=$(printf '%q' "$systemd_unit") \
      $(printf '%q' "$remote_dir/scripts/overnight-rebuild.sh")"
  exit 0
fi

echo "Switching $host to $remote_dir#$flake_attr"
ssh -t "$host" "sudo $rebuild_cmd"
