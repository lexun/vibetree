{
  inputs = {
    nixpkgs.url = "github:cachix/devenv-nixpkgs/rolling";
    devenv.url = "github:cachix/devenv";
    devenv.inputs.nixpkgs.follows = "nixpkgs";
  };

  nixConfig = {
    extra-trusted-public-keys = "devenv.cachix.org-1:w1cLUi8dv3hnoSPGAuibQv+f9TZLr6cv/Hm9XgU50cw=";
    extra-substituters = "https://devenv.cachix.org";
  };

  outputs =
    {
      self,
      nixpkgs,
      devenv,
      systems,
      ...
    }@inputs:
    let
      devenvEnabled = (builtins.getEnv "DEVENV_ENABLED") == "true";
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forEachSupportedSystem =
        fn:
        nixpkgs.lib.genAttrs supportedSystems (
          system:
          fn {
            inherit system;
            pkgs = import nixpkgs { inherit system; };
          }
        );
    in
    {
      formatter = forEachSupportedSystem ({ pkgs, ... }: pkgs.nixfmt-rfc-style);

      packages = forEachSupportedSystem (
        { pkgs, ... }:
        {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "vibetree";
            version = "0.1.0";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;

            # Skip integration tests in Nix build (they require git and file system access)
            cargoTestFlags = [ "--lib" ];

            buildInputs = with pkgs; [
              openssl
              pkg-config
            ];

            nativeBuildInputs = with pkgs; [
              pkg-config
              perl
            ];
          };
        }
      );

      overlays.default = final: prev: {
        vibetree = self.packages.${final.system}.default;
      };

      devShells = forEachSupportedSystem (
        { pkgs, ... }:
        pkgs.lib.optionalAttrs devenvEnabled {
          default = devenv.lib.mkShell {
            inherit inputs pkgs;
            modules = [
              # https://devenv.sh/reference/options/
              {
                languages.rust.enable = true;
                packages = with pkgs; [ ];
              }
            ];
          };
        }
      );
    };
}
