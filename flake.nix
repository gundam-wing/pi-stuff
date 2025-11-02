{
  description = "A development environment for nodejs";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/2237beb4d9789f6b374542a93959c9c3aea04678";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            nodejs_22
          ];

          shellHook = ''
            echo "Node.js $(node --version) is available"
            echo "npm $(npm --version) is available"
          '';
        };
      }
    );
}
