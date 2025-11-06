{
  config,
  pkgs,
  lib,
  ...
}:
let
  user = "guest";
  password = "***";
  SSID = "YourNetworkName";
  SSIDpassword = "***";
  interface = "wlan0";
  hostname = "myhostname";
in
{
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
      networks."${SSID}".psk = SSIDpassword;
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

  # Enable passwordless sudo.
  security.sudo.extraRules = [
    {
      users = [ user ];
      commands = [
        {
          command = "ALL";
          options = [ "NOPASSWD" ];
        }
      ];
    }
  ];

  services.xserver = {
    enable = true;
    displayManager.lightdm.enable = true;
    desktopManager.xfce.enable = true;
  };

  services.openssh.enable = true;

  users = {
    mutableUsers = false;
    users."${user}" = {
      isNormalUser = true;
      password = password;
      extraGroups = [
        "wheel"
        "video" # Required for camera access
      ];
      openssh.authorizedKeys.keys = [
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHPcdh0STKBGoNZMyTqQWjmrlkfMNkmRRq/Ki1PQcefB spenceropope@gmail.com"
      ];
    };
  };

  hardware.enableRedistributableFirmware = true;

  # Enable flakes
  nix.settings.experimental-features = [
    "nix-command"
    "flakes"
  ];

  system.stateVersion = "23.11";
}

# run with: sudo nixos-rebuild switch --flake /etc/nixos#myhostname
