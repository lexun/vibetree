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
            version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;

            # Disable all tests in the output derivation
            doCheck = false;

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

      checks = forEachSupportedSystem (
        { pkgs, ... }:
        {
          vibetree-tests = self.packages.${pkgs.system}.default.overrideAttrs (old: {
            doCheck = true;
            cargoTestFlags = [ "--lib" ];
            nativeBuildInputs = (old.nativeBuildInputs or [ ]) ++ [ pkgs.git ];
            buildPhase = "true";
            installPhase = "mkdir -p $out";
          });
        }
      );

      overlays.default = final: prev: {
        vibetree = self.packages.${final.stdenv.hostPlatform.system}.default;
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
                packages = with pkgs; [
                  cargo-edit
                  just
                ];
                git-hooks.hooks.single-line-commit = {
                  enable = true;
                  name = "single-line commit";
                  entry = "bash -c 'test $(grep -cv \"^#\" \"$1\") -le 1' --";
                  stages = [ "commit-msg" ];
                };
              }
            ];
          };
        }
      );
    };
}
