{
  config,
  lib,
  monitor-src,
  monitorRevision,
  pkgs,
  web-src,
  ...
}:

let
  cfg = config.services.pi-camera-monitor;

  monitorWeb = pkgs.buildNpmPackage {
    pname = "pi-camera-monitor-web";
    version = "0.1.0";
    src = web-src;
    nodejs = pkgs.nodejs_24;
    npmDepsHash = "sha256-Iu5nBk7BEpRGFaodh0dSMPMqXQIbrfVlyaSawCFKCu0=";

    npmBuildScript = "build";
    installPhase = ''
      runHook preInstall
      mkdir -p "$out/share/pi-camera-monitor/web"
      cp -r dist/. "$out/share/pi-camera-monitor/web/"
      runHook postInstall
    '';
  };

  monitorService = pkgs.rustPlatform.buildRustPackage {
    pname = "pi-camera-monitor";
    version = "0.1.0";
    src = monitor-src;
    cargoLock = {
      lockFile = "${monitor-src}/Cargo.lock";
      # crates.io's API download endpoint returns 403 to Nix's curl User-Agent;
      # fetch from the CDN instead.
      extraRegistries = {
        "https://github.com/rust-lang/crates.io-index" = "https://static.crates.io/crates";
      };
    };

    postInstall = ''
      mv "$out/bin/monitor" "$out/bin/pi-camera-monitor"
    '';

    meta = {
      description = "Supervised HLS and WebRTC camera service for Raspberry Pi";
      mainProgram = "pi-camera-monitor";
    };
  };
in
{
  options.services.pi-camera-monitor = {
    enable = lib.mkEnableOption "the private Raspberry Pi camera monitor";

    port = lib.mkOption {
      type = lib.types.port;
      default = 8080;
      description = "TCP port exposed on the Tailscale interface.";
    };

    width = lib.mkOption {
      type = lib.types.ints.positive;
      default = 1280;
      description = "Captured video width in pixels.";
    };

    height = lib.mkOption {
      type = lib.types.ints.positive;
      default = 720;
      description = "Captured video height in pixels.";
    };

    frameRate = lib.mkOption {
      type = lib.types.ints.positive;
      default = 15;
      description = "Captured frames per second.";
    };

    bitrate = lib.mkOption {
      type = lib.types.ints.positive;
      default = 2500000;
      description = "H.264 encoder bitrate in bits per second.";
    };

    motion = {
      maxEvents = lib.mkOption {
        type = lib.types.ints.positive;
        default = 48;
        description = "Maximum number of motion events kept on disk.";
      };

      maxBytes = lib.mkOption {
        type = lib.types.ints.positive;
        default = 16777216;
        description = "Maximum total size of stored motion JPEGs, in bytes.";
      };

      threshold = lib.mkOption {
        type = lib.types.float;
        default = 0.02;
        description = "Fraction of analysis pixels that must change to count as motion.";
      };

      pixelFloor = lib.mkOption {
        type = lib.types.ints.between 0 255;
        default = 25;
        description = "Per-pixel luma delta below which change is treated as noise.";
      };

      cooldownMs = lib.mkOption {
        type = lib.types.ints.between 0 3600000;
        default = 3000;
        description = "Minimum time between motion events, in milliseconds.";
      };

      settleSecs = lib.mkOption {
        type = lib.types.ints.between 0 60;
        default = 5;
        description = "Seconds to ignore motion after the stream becomes live.";
      };

      roi = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = "0,0,0.6,1";
        description = "Normalized analysis box x,y,w,h in 0-1. Null scores the full frame.";
      };
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = pkgs ? rpicam-apps;
        message = "pi-camera-monitor requires the rpicam-apps package overlay";
      }
    ];

    users.groups.pi-camera-monitor = { };
    users.users.pi-camera-monitor = {
      isSystemUser = true;
      group = "pi-camera-monitor";
      extraGroups = [ "video" ];
    };

    systemd.services.pi-camera-monitor = {
      description = "Raspberry Pi camera HLS and WebRTC monitor";
      wantedBy = [ "multi-user.target" ];
      after = [
        "network.target"
        "tailscaled.service"
      ];
      wants = [ "tailscaled.service" ];

      environment = {
        MONITOR_BIND = "0.0.0.0:${toString cfg.port}";
        MONITOR_HLS_DIR = "/run/pi-camera-monitor/hls";
        MONITOR_WEB_DIR = "${monitorWeb}/share/pi-camera-monitor/web";
        MONITOR_FFMPEG_BIN = "${pkgs.ffmpeg}/bin/ffmpeg";
        MONITOR_FRAME_RATE = toString cfg.frameRate;
        MONITOR_REVISION = monitorRevision;
        MONITOR_MOTION_DIR = "/var/lib/pi-camera-monitor/motion";
        MONITOR_MOTION_MAX_EVENTS = toString cfg.motion.maxEvents;
        MONITOR_MOTION_MAX_BYTES = toString cfg.motion.maxBytes;
        MONITOR_MOTION_THRESHOLD = toString cfg.motion.threshold;
        MONITOR_MOTION_PIXEL_FLOOR = toString cfg.motion.pixelFloor;
        MONITOR_MOTION_COOLDOWN_MS = toString cfg.motion.cooldownMs;
        MONITOR_MOTION_SETTLE_SECS = toString cfg.motion.settleSecs;
        MONITOR_MOTION_ROI = lib.optionalString (cfg.motion.roi != null) cfg.motion.roi;
        MONITOR_CAPTURE_COMMAND = lib.concatStringsSep " " [
          "${pkgs.rpicam-apps}/bin/rpicam-vid"
          "--camera 0"
          "--timeout 0"
          "--width ${toString cfg.width}"
          "--height ${toString cfg.height}"
          "--framerate ${toString cfg.frameRate}"
          "--codec h264"
          "--inline"
          "--intra ${toString cfg.frameRate}"
          "--bitrate ${toString cfg.bitrate}"
          "--nopreview"
          "--output -"
        ];
      };

      serviceConfig = {
        ExecStart = lib.getExe monitorService;
        User = "pi-camera-monitor";
        Group = "pi-camera-monitor";
        Restart = "on-failure";
        RestartSec = "3s";
        RuntimeDirectory = "pi-camera-monitor";
        RuntimeDirectoryMode = "0700";
        StateDirectory = "pi-camera-monitor";
        StateDirectoryMode = "0700";
        UMask = "0077";

        NoNewPrivileges = true;
        PrivateDevices = false;
        PrivateTmp = true;
        ProtectClock = true;
        ProtectControlGroups = true;
        ProtectHome = true;
        ProtectKernelLogs = true;
        ProtectKernelModules = true;
        ProtectKernelTunables = true;
        ProtectSystem = "strict";
        RestrictAddressFamilies = [
          "AF_INET"
          "AF_INET6"
          "AF_UNIX"
        ];
        RestrictNamespaces = true;
        RestrictRealtime = true;
        RestrictSUIDSGID = true;
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
        CapabilityBoundingSet = "";
      };
    };

    networking.firewall.interfaces.tailscale0.allowedTCPPorts = [ cfg.port ];

    environment.systemPackages = [
      monitorService
      pkgs.ffmpeg
      pkgs.rpicam-apps
    ];
  };
}
