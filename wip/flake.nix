{
  description = "Raspberry Pi 4 NixOS configuration with Camera Module 3";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    nixos-hardware.url = "github:NixOS/nixos-hardware/master";
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # nixos-raspi-camera.url = "github:sergei-mironov/nixos-raspi-camera";
    # nixos-raspi-camera.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      self,
      nixpkgs,
      nixos-hardware,
      home-manager,
      # nixos-raspi-camera,
      ...
    }:
    {
      nixosConfigurations.myhostname = nixpkgs.lib.nixosSystem {
        system = "aarch64-linux";
        modules = [
          # nixos-raspi-camera.nixosModules.raspi-camera
          nixos-hardware.nixosModules.raspberry-pi-4
          ./configuration.nix
          home-manager.nixosModules.home-manager
          {
            home-manager.useGlobalPkgs = true;
            home-manager.useUserPackages = true;
            home-manager.users.guest = import ./home.nix;
          }
        ];
      };
    };
}
