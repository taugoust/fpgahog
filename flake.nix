{
  description = "fpgahog: coordinate resource usage on shared Linux hosts";

  nixConfig.extra-substituters = [ "https://nix-community.cachix.org" ];
  nixConfig.extra-trusted-public-keys = [
    "nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="
  ];

  inputs = {
    nixpkgs.url = "nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-parts, fenix, ... }@inputs:
    (flake-parts.lib.evalFlakeModule { inherit inputs; } {
      systems = [ "x86_64-linux" "aarch64-linux" ];
      perSystem = { system, self', pkgs, ... }:
        let
          fenixPkgs = fenix.packages.${system};
          toolchain = fenixPkgs.stable;
          rustToolchain = fenixPkgs.combine [
            toolchain.cargo
            toolchain.rustc
            toolchain.rust-src
            toolchain.rust-std
            toolchain.clippy
            toolchain.rustfmt
          ];
          rustPlatform = pkgs.makeRustPlatform {
            cargo = rustToolchain;
            rustc = rustToolchain;
          };
          cargoMetadata = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package;
          fpgahog = rustPlatform.buildRustPackage {
            pname = cargoMetadata.name;
            version = cargoMetadata.version;
            src = pkgs.lib.fileset.toSource {
              root = ./.;
              fileset = pkgs.lib.fileset.unions [
                ./Cargo.toml
                ./Cargo.lock
                ./LICENSE
                (pkgs.lib.fileset.maybeMissing ./src)
                (pkgs.lib.fileset.maybeMissing ./tests)
              ];
            };
            cargoLock.lockFile = ./Cargo.lock;
            postInstall = ''
              ln -s fpgahog $out/bin/hosthog
            '';
            meta = with pkgs.lib; {
              description = "Coordinate FPGA and host resource usage on shared Linux hosts";
              homepage = "https://github.com/halalboro/fpgahog";
              license = licenses.mit;
              platforms = platforms.linux;
              mainProgram = "fpgahog";
            };
          };
        in {
<<<<<<< HEAD
          packages.default = fpgahog;
          checks = { package = fpgahog; };
=======
          packages.default = hosthog;
          checks = {
            package = pkgs.runCommand "hosthog-package-check" { } ''
              test -x ${hosthog}/bin/hosthog
              ${hosthog}/bin/hosthog --version | grep -Fx "hosthog ${cargoMetadata.version}"
              touch $out
            '';
          };
>>>>>>> 7ca002b (Complete Nix runtime packaging)
          devShells.default = pkgs.mkShell {
            inputsFrom = [ fpgahog ];
            RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
            buildInputs = [ rustToolchain fenixPkgs.rust-analyzer pkgs.just ];
          };
        };
    }).config.flake;
}
