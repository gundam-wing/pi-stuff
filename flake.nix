{
  description = "Raspberry Pi 4 NixOS camera monitor";

  inputs = {
    # Pinned deliberately: bumping nixpkgs/hardware/home-manager on the Pi can
    # rebuild the kernel and takes a long overnight build. Keep these in sync.
    # This revision is after the crates.io UA / static.crates.io cargo fetch fix
    # (nixpkgs f830e6112b4d, 2026-05-27), so newer Rust deps can vendor again.
    nixpkgs.url = "github:NixOS/nixpkgs/801bef6abd86b91e51083066b83fb354a11fc640";
    nixos-hardware = {
      url = "github:NixOS/nixos-hardware/44d95795ee2d475b3d687325e26dcf4ca9104557";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    home-manager = {
      url = "github:nix-community/home-manager/caa6dc59c445ef78b82b8684103d02095158fd82";
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
