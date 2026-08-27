{
  config,
  pkgs,
  lib,
  ...
}:
let
  user = "guest";
  interface = "wlan0";
  hostname = "pi-camera";
in
{
  sops = {
    defaultSopsFile = ./secrets/pi.yaml;
    age.sshKeyPaths = [ "/etc/ssh/ssh_host_ed25519_key" ];

    secrets = {
      wifi_ssid = { };
      wifi_password = { };
      guest_password_hash.neededForUsers = true;
    };

    # Rendered at activation so the SSID is not a Nix attribute name.
    templates."wireless.conf" = {
      content = ''
        network={
          ssid="${config.sops.placeholder.wifi_ssid}"
          psk="${config.sops.placeholder.wifi_password}"
          key_mgmt=WPA-PSK WPA-EAP SAE FT-PSK FT-EAP FT-SAE
        }
      '';
      mode = "0400";
      restartUnits = [ "wpa_supplicant-${interface}.service" ];
    };
  };

  boot = {
    kernelPackages = pkgs.linuxKernel.packages.linux_rpi4;
    initrd.availableKernelModules = [
      "xhci_pci"
      "usbhid"
      "usb_storage"
    ];
    loader = {
      grub.enable = false;
      generic-extlinux-compatible.enable = true;
    };
  };

  fileSystems = {
    "/" = {
      device = "/dev/disk/by-label/NIXOS_SD";
      fsType = "ext4";
      options = [ "noatime" ];
    };
  };

  networking = {
    hostName = hostname;
    wireless = {
      enable = true;
      # Keep generated config so extraConfigFiles are included; the network
      # block itself comes from the sops template.
      extraConfig = "# SSID and PSK are loaded from extraConfigFiles.";
      extraConfigFiles = [ config.sops.templates."wireless.conf".path ];
      interfaces = [ interface ];
    };
    interfaces."${interface}".ipv4.addresses = [
      {
        address = "10.0.1.200";
        prefixLength = 24;
      }
    ];
    defaultGateway = "10.0.1.1";
    nameservers = [
      "1.1.1.1"
      "8.8.8.8"
    ];
  };

  environment.systemPackages = with pkgs; [
    vim
    git
    xterm
  ];

  fonts = {
    packages = with pkgs; [
      jetbrains-mono
    ];
    fontconfig = {
      defaultFonts = {
        monospace = [ "JetBrains Mono" ];
        sansSerif = [ "JetBrains Mono" ];
        serif = [ "JetBrains Mono" ];
      };
    };
  };

  # Enable GPU acceleration
  hardware.raspberry-pi."4".fkms-3d.enable = true;

  security.sudo.wheelNeedsPassword = true;

  services.xserver = {
    enable = true;
    displayManager.lightdm.enable = true;
    desktopManager.xfce.enable = true;
  };

  services.openssh = {
    enable = true;
    openFirewall = false;
    settings = {
      AllowTcpForwarding = false;
      GatewayPorts = "no";
      PasswordAuthentication = false;
      KbdInteractiveAuthentication = false;
      PermitRootLogin = "no";
      PermitTunnel = false;
      X11Forwarding = false;
    };
  };
  services.tailscale = {
    enable = true;
    openFirewall = true;
  };
  services.pi-camera-monitor.enable = true;

  users = {
    mutableUsers = false;
    users."${user}" = {
      isNormalUser = true;
      hashedPasswordFile = config.sops.secrets.guest_password_hash.path;
      extraGroups = [
        "wheel"
        "video" # Required for camera access
      ];
      # Public keys are safe to commit; use a key dedicated to this host.
      openssh.authorizedKeys.keys = [
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHPcdh0STKBGoNZMyTqQWjmrlkfMNkmRRq/Ki1PQcefB"
      ];
    };
  };

  # Administrative SSH is reachable over Tailscale and the trusted Wi-Fi LAN.
  networking.firewall.interfaces.tailscale0.allowedTCPPorts = [ 22 ];
  networking.firewall.interfaces."${interface}".allowedTCPPorts = [ 22 ];

  hardware.enableRedistributableFirmware = true;

  # Enable flakes
  nix.settings = {
    experimental-features = [
      "nix-command"
      "flakes"
    ];
    # Native builds can otherwise exhaust RAM and make SSH unresponsive.
    max-jobs = 2;
    cores = 2;
  };

  system.stateVersion = "23.11";
}

# run with: sudo nixos-rebuild switch --flake /etc/nixos#myhostname
