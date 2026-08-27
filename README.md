# Raspberry Pi camera monitor

A private, live HLS viewer for a Raspberry Pi 4 and Camera Module 3 running
NixOS. The monitor uses `rpicam-vid` for H.264 capture, FFmpeg for HLS muxing,
a Rust service for supervision and HTTP, and a TypeScript web viewer.

## Deploy

The root flake assembles the application and the NixOS modules under `nixos/`.
Deployment secrets are encrypted in `nixos/secrets/pi.yaml` for both the Pi host
SSH key and the administrator SSH key. Before the first rebuild, generate a
yescrypt password hash:

```sh
nix shell nixpkgs#mkpasswd -c mkpasswd -m yescrypt
```

Then decrypt and edit the secrets file using the administrator SSH private key:

```sh
export SOPS_AGE_SSH_PRIVATE_KEY_FILE="$HOME/.ssh/id_ed25519"
nix run nixpkgs#sops -- nixos/secrets/pi.yaml
```

Replace both `CHANGE_ME` values and paste the generated hash into
`guest_password_hash`. Also replace the public key in `nixos/configuration.nix`
with a key dedicated to this host. Public keys are not secret and can remain in
Git. The password is required for `sudo`; SSH remains public-key-only.

The `monitor/` and `web/` directories are Nix flake inputs, so they have to be
on the Pi. From this machine, copy the tree and switch the running generation:

```sh
./scripts/deploy.sh
```

That rsyncs into `~/pi-stuff` on `guest@10.0.1.200`, skipping Git metadata,
build artifacts, and the large `images/` tree, then runs `nixos-rebuild switch`
over SSH. Sudo on the Pi prompts for the `guest` password. Copy without
switching with `./scripts/deploy.sh --sync-only`. Override the SSH target with
`PI_HOST=guest@pi-camera` when Tailscale hostname verification is set up.

If you are already on the Pi:

```sh
cd ~/pi-stuff
sudo nixos-rebuild switch --max-jobs 2 --cores 2 --flake .#myhostname
```

The Wi-Fi SSID and password stay in `nixos/secrets/pi.yaml` and are rendered
under `/run` at activation, so they never appear as Nix attribute names. The
Pi stays at `10.0.1.200`. The host SSH private key in
`/etc/ssh/ssh_host_ed25519_key` decrypts secrets during boot.

The service starts automatically. Useful checks:

```sh
systemctl status pi-camera-monitor
journalctl -u pi-camera-monitor -f
curl http://127.0.0.1:8080/health
```

The rolling HLS playlist and segments live under `/run/pi-camera-monitor`, so
they are bounded and do not write continuously to the SD card. Motion stills are
kept in a capped ring under `/var/lib/pi-camera-monitor/motion` and shown below
the live viewer. Raise `services.pi-camera-monitor.motion.threshold` or set
`motion.roi` if trees or lighting chatter fill the gallery.

The flake pins the same NixOS, hardware, and Home Manager revisions as the
deployed Pi. This prevents an application change from unexpectedly rebuilding
the kernel. Update those inputs deliberately. Native builds are limited to two
jobs/two cores to preserve enough memory for SSH; a remote aarch64 builder or
binary cache is the next step if builds become frequent.

## Connect through Tailscale

Tailscale provides the encrypted private network; it does not host or relay the
web application under normal operation. No router ports need to be opened.

1. From a physical console, run `sudo tailscale up`.
2. Open the displayed URL once and approve the Pi.
3. In the Tailscale admin console, disable key expiry for this always-on Pi.
4. Install Tailscale on the phone and sign in to the same account.
5. With Tailscale connected, open `http://pi-camera:8080`.

From the Mac, connect to the Pi with:

```sh
ssh guest@pi-camera
```

If local hostname resolution is unavailable, use `ssh guest@10.0.1.200`.

TCP ports 22 and 8080 are allowed on `tailscale0`; only SSH port 22 is also
allowed on the trusted Wi-Fi interface. This permits development access from
the local network when Tailscale is unavailable. SSH password and root login,
forwarding, and tunneling remain disabled. Members of the same tailnet can view
the MVP stream, so remove untrusted members or add a Tailscale access-control
policy before sharing the tailnet.

## Development

```sh
cd monitor
cargo test
cargo clippy --all-targets -- -D warnings

cd ../web
npm ci
npm run build
```

The Rust service accepts these environment variables:

- `MONITOR_BIND` (default `127.0.0.1:8080`)
- `MONITOR_HLS_DIR` (default `/run/pi-camera-monitor/hls`)
- `MONITOR_WEB_DIR` (default `./web/dist`)
- `MONITOR_CAPTURE_COMMAND` (defaults to 720p, 15 fps H.264 from `rpicam-vid`)
- `MONITOR_FFMPEG_BIN` (default `ffmpeg`)
- `MONITOR_MOTION_DIR` (default `/var/lib/pi-camera-monitor/motion`)
- `MONITOR_MOTION_MAX_EVENTS` (default `48`)
- `MONITOR_MOTION_MAX_BYTES` (default `16777216`)
- `MONITOR_MOTION_THRESHOLD` (default `0.02`)
- `MONITOR_MOTION_PIXEL_FLOOR` (default `25`)
- `MONITOR_MOTION_COOLDOWN_MS` (default `3000`)
- `MONITOR_MOTION_SETTLE_SECS` (default `5`)
- `MONITOR_MOTION_ROI` (optional `x,y,w,h` in 0–1, for example `0,0,0.6,1`)
