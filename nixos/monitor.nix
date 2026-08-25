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
    cargoLock.lockFile = "${monitor-src}/Cargo.lock";

    postInstall = ''
      mv "$out/bin/monitor" "$out/bin/pi-camera-monitor"
    '';

    meta = {
      description = "Supervised HLS camera service for Raspberry Pi";
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
      description = "Raspberry Pi camera HLS monitor";
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
