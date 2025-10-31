# NixOS Raspberry Pi 4 Setup Guide

Complete walkthrough for setting up WiFi, SSH, static IP, desktop environment, and SSH keys on a fresh NixOS Raspberry Pi 4 installation.

---

## Initial WiFi Connection (from NixOS machine)

After booting NixOS and starting a root shell with `sudo -i`:

### Using NetworkManager (nmcli)
```bash
# List available WiFi networks
nmcli device wifi list

# Connect to a network (SSID is the network name like "YourNetworkName")
nmcli device wifi connect "YourNetworkName" password "YourPassword"

# Check connection status
nmcli device status
```

**Note:** The network name is the SSID (human-readable name), not the BSSID (MAC address). Use quotes if the name contains spaces or special characters.

---

## Setting Up SSH Access

### On the NixOS machine:

```bash
# Set root password
passwd

# Start SSH service (one-time, until reboot)
systemctl start sshd

# Check if SSH is running
systemctl status sshd

# Find your IP address
hostname -I
# Example output: 10.0.1.15 2603...
# The first address (10.0.1.15) is your local IP
```

### From your Mac:

```bash
# SSH into the NixOS machine
ssh root@10.0.1.15

# If password authentication fails, you may need to enable it on NixOS first
```

### Troubleshooting SSH password issues (on NixOS machine):

```bash
# Check SSH configuration
cat /etc/ssh/sshd_config | grep PermitRootLogin

# If needed, edit the config
nano /etc/ssh/sshd_config
# Change to: PermitRootLogin yes

# Restart SSH
systemctl restart sshd
```

---

## Complete NixOS Configuration File

Instead of manually configuring WiFi and SSH each time, create a proper NixOS configuration at `/etc/nixos/configuration.nix`:

```nix
{ config, pkgs, lib, ... }:

let
  user = "****";
  password = "****";
  SSID = "****";
  SSIDpassword = "****";
  interface = "wlan0";
  hostname = "myhostname";
in {

  boot = {
    kernelPackages = pkgs.linuxKernel.packages.linux_rpi4;
    initrd.availableKernelModules = [ "xhci_pci" "usbhid" "usb_storage" ];
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
    # Static IP configuration
    interfaces."${interface}".ipv4.addresses = [{
      address = "10.0.1.200";  # High number to avoid DHCP conflicts
      prefixLength = 24;
    }];
    defaultGateway = "10.0.1.1";
    nameservers = [ "1.1.1.1" "8.8.8.8" ];
  };

  environment.systemPackages = with pkgs; [ vim ];

  # Enable SSH
  services.openssh.enable = true;

  # User configuration
  users = {
    mutableUsers = false;  # Passwords managed via config file
    users."${user}" = {
      isNormalUser = true;
      password = password;
      extraGroups = [ "wheel" ];  # Allows sudo access
    };
  };

  hardware.enableRedistributableFirmware = true;
  system.stateVersion = "23.11";
}
```

### Apply the configuration:

```bash
# Edit the config file
sudo vim /etc/nixos/configuration.nix

# Apply changes and switch to new configuration
sudo nixos-rebuild switch

# Reboot to ensure everything works on boot
reboot
```

---

## Finding Network Configuration Values

To determine the correct values for static IP configuration (run on NixOS machine):

```bash
# View network interface information
ip addr show wlan0

# Find default gateway (router IP)
ip route show default
# Example output: default via 10.0.1.1 dev wlan0 proto dhcp src 10.0.1.16 metric 3003
# Gateway is 10.0.1.1

# Check current DNS servers
cat /etc/resolv.conf
```

**Choosing a static IP:**
- Stay in the same subnet (e.g., 10.0.1.x)
- Choose a high number (like 10.0.1.200) to avoid DHCP range conflicts
- Most routers use DHCP ranges like 10.0.1.2-10.0.1.99

---

## Adding a Desktop Environment (XFCE)

Add to your `/etc/nixos/configuration.nix`:

```nix
# Enable GPU acceleration for Raspberry Pi 4
hardware.raspberry-pi."4".fkms-3d.enable = true;

# Enable XFCE desktop (lightweight, good for Pi)
services.xserver = {
  enable = true;
  displayManager.lightdm.enable = true;
  desktopManager.xfce.enable = true;
};
```

Then rebuild:
```bash
sudo nixos-rebuild switch
reboot
```

### XFCE Keyboard Shortcuts (useful without a mouse):

- `Alt + F1` - Open applications menu
- `Alt + F2` - Run dialog (type application names)
- `Alt + Tab` - Switch windows
- `Alt + F4` - Close window
- `Ctrl + Alt + T` - Open terminal
- `Tab` / `Arrow keys` - Navigate menus
- `Enter` - Select/activate
- `Esc` - Cancel/close

---

## Setting Up SSH Key Authentication

### On your Mac:

```bash
# Check if you have an SSH key
ls ~/.ssh/id_ed25519.pub

# If not, generate one
ssh-keygen -t ed25519 -C "your_email@example.com"
# Press Enter to accept defaults

# Display your public key
cat ~/.ssh/id_ed25519.pub
# Copy the entire output (starts with ssh-ed25519 AAAA...)
```

### On NixOS machine:

Edit `/etc/nixos/configuration.nix` and add the key to your user configuration:

```nix
users.users."${user}" = {
  isNormalUser = true;
  password = password;
  extraGroups = [ "wheel" ];
  openssh.authorizedKeys.keys = [
    "ssh-ed25519 AAAA... your-public-key-here"
  ];
};
```

Apply changes:
```bash
sudo nixos-rebuild switch
```

### Test from your Mac:

```bash
ssh guest@10.0.1.200
# Should connect without password!
```

### Optional - Disable password authentication (more secure):

Add to `/etc/nixos/configuration.nix`:
```nix
services.openssh.settings.PasswordAuthentication = false;
```

Then rebuild again.

---

## Common Troubleshooting Commands

### On NixOS machine:

```bash
# Check SSH service status
systemctl status sshd

# Check if SSH is listening on port 22
ss -tlnp | grep 22

# Check firewall status
systemctl status firewall

# View current IP address
hostname -I

# Test internet connectivity
ping -c 3 1.1.1.1

# Find wireless interface name
ip link show
```

### From Mac:

```bash
# Verbose SSH connection (see where it hangs)
ssh -v guest@10.0.1.200

# Find devices on local network
arp -a

# Or with nmap (if installed)
nmap -sn 192.168.1.0/24
```

---

## Key Concepts

### Command Locations:
- **On NixOS machine** (via direct access or SSH): System configuration, network setup, service management
- **On Mac** (your local machine): SSH connections, generating SSH keys, network scanning

### Important NixOS Concepts:
- **Declarative configuration**: Everything in `/etc/nixos/configuration.nix`
- **Immutable users** (`mutableUsers = false`): Passwords can only be changed via config file
- **nixos-rebuild switch**: Applies configuration changes immediately
- **Services persistence**: Services enabled in config start automatically on boot

### File Locations:
- Main config: `/etc/nixos/configuration.nix`
- SSH config: `/etc/ssh/sshd_config`
- DNS config: `/etc/resolv.conf`

---

## Quick Reference: Your Final Configuration

- **Static IP**: 10.0.1.200
- **Gateway**: 10.0.1.1
- **DNS**: 1.1.1.1, 8.8.8.8
- **Username**: guest (in wheel group, can use sudo)
- **Desktop**: XFCE with LightDM
- **SSH**: Enabled with key authentication
- **WiFi**: Auto-connects on bo

## Resources

- https://nix.dev/tutorials/nixos/installing-nixos-on-a-raspberry-pi.html
- https://mtlynch.io/nixos-pi4/