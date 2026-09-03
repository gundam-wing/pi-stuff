{
  description = "Raspberry Pi 4 NixOS camera monitor";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/b3d51a0365f6695e7dd5cdf3e180604530ed33b4";
    nixos-hardware.url = "github:NixOS/nixos-hardware/2e85ae1b7030df39269d29118b1f74944d0c8f15";
    home-manager = {
      url = "github:nix-community/home-manager/af119feb17cb242398e0fb97f92b867d25882522";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    sops-nix = {
      url = "github:Mic92/sops-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    monitor-src = {
      url = "path:./monitor";
      flake = false;
    };
    web-src = {
      url = "path:./web";
      flake = false;
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      nixos-hardware,
      home-manager,
      sops-nix,
      monitor-src,
      web-src,
      ...
    }:
    let
      inherit (nixpkgs) lib;
      revFile = self + "/REVISION";
      fromFile =
        if builtins.pathExists revFile then lib.removeSuffix "\n" (builtins.readFile revFile) else "";
      monitorRevision =
        if fromFile != "" then
          fromFile
        else
          self.dirtyShortRev or self.shortRev or "unknown";
    in
    {
      nixosConfigurations.myhostname = nixpkgs.lib.nixosSystem {
        system = "aarch64-linux";
        specialArgs = {
          inherit monitor-src monitorRevision web-src;
        };
        modules = [
          nixos-hardware.nixosModules.raspberry-pi-4
          {
            system.configurationRevision = self.rev or self.dirtyRev or (
              if monitorRevision == "unknown" then null else monitorRevision
            );
            nixpkgs.overlays = [
              (
                final: _prev: {
                  rpicam-apps = final.callPackage ./nixos/rpicam-apps.nix { };
                }
              )
              (import ./nixos/overlays/crates-io-static.nix)
            ];
          }
          ./nixos/configuration.nix
          ./nixos/camera.nix
          ./nixos/monitor.nix
          home-manager.nixosModules.home-manager
          sops-nix.nixosModules.sops
          {
            home-manager.useGlobalPkgs = true;
            home-manager.useUserPackages = true;
            home-manager.users.guest = import ./nixos/home.nix;
          }
        ];
      };
    };
}
