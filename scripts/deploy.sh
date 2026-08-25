#!/usr/bin/env bash
set -euo pipefail

# Copy this flake to the Pi and switch the running NixOS generation.
# The monitor/ and web/ trees are flake inputs, so they must be on the Pi.
#
# Usage:
#   ./scripts/deploy.sh              # rsync, then nixos-rebuild switch
#   ./scripts/deploy.sh --sync-only  # rsync only
#
# Optional environment:
#   PI_HOST       SSH target (default: guest@10.0.1.200)
#   PI_DIR        Remote flake path (default: /home/guest/pi-stuff)
#   FLAKE_ATTR    nixosConfigurations attr (default: myhostname)
#   PI_JOBS       nix max-jobs (default: 2)
#   PI_CORES      nix cores (default: 2)

root="$(cd "$(dirname "$0")/.." && pwd)"
host="${PI_HOST:-guest@10.0.1.200}"
remote_dir="${PI_DIR:-/home/guest/pi-stuff}"
flake_attr="${FLAKE_ATTR:-myhostname}"
jobs="${PI_JOBS:-2}"
cores="${PI_CORES:-2}"
sync_only=0

if [[ "${1:-}" == "--sync-only" ]]; then
  sync_only=1
elif [[ "${1:-}" != "" ]]; then
  echo "usage: $0 [--sync-only]" >&2
  exit 2
fi

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

if [[ "$sync_only" -eq 1 ]]; then
  echo "Copied. Rebuild later with:"
  echo "  ssh -t $host 'sudo nixos-rebuild switch --max-jobs $jobs --cores $cores --flake $remote_dir#$flake_attr'"
  exit 0
fi

echo "Switching $host to $remote_dir#$flake_attr"
ssh -t "$host" "sudo nixos-rebuild switch --max-jobs $jobs --cores $cores --flake $remote_dir#$flake_attr"
