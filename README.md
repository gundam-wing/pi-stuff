# Raspberry Pi camera monitor

A private, live HLS viewer for a Raspberry Pi 4 and Camera Module 3 running
NixOS. The monitor uses `rpicam-vid` for H.264 capture, FFmpeg for HLS muxing,
a Rust service for supervision and HTTP, and a TypeScript web viewer.

## Deploy

The root flake assembles the application and the NixOS modules under `wip/`.
Before rebuilding, create the Wi-Fi environment file on the Pi without adding
it to Git:

```sh
sudo install -m 0600 /dev/null /etc/nixos/wireless.env
sudoedit /etc/nixos/wireless.env
```

Its content is:

```text
SSID_PASSWORD=your-real-wifi-password
```

Copy this repository to the Pi (the `monitor/` and `web/` directories are Nix
build inputs), then apply the configuration:

```sh
sudo nixos-rebuild switch --max-jobs 2 --cores 2 --flake .#myhostname
```

The service starts automatically. Useful checks:

```sh
systemctl status pi-camera-monitor
journalctl -u pi-camera-monitor -f
curl http://127.0.0.1:8080/health
```

The rolling HLS playlist and segments live under `/run/pi-camera-monitor`, so
they are bounded and do not write continuously to the SD card.

The flake pins the same NixOS, hardware, and Home Manager revisions as the
deployed Pi. This prevents an application change from unexpectedly rebuilding
the kernel. Update those inputs deliberately. Native builds are limited to two
jobs/two cores to preserve enough memory for SSH; a remote aarch64 builder or
binary cache is the next step if builds become frequent.

## Connect through Tailscale

Tailscale provides the encrypted private network; it does not host or relay the
web application under normal operation. No router ports need to be opened.

1. On the Pi, run `sudo tailscale up`.
2. Open the displayed URL once and approve the Pi.
3. In the Tailscale admin console, disable key expiry for this always-on Pi.
4. Install Tailscale on the phone and sign in to the same account.
5. With Tailscale connected, open `http://myhostname:8080`.

Only TCP port 8080 on `tailscale0` is allowed by the NixOS firewall. Members of
the same tailnet can view the MVP stream, so remove untrusted members or add a
Tailscale access-control policy before sharing the tailnet.

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
