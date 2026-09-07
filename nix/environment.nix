{ inputs, system }:
let
  inherit (inputs.nixpkgs) lib;
  target = import ./target.nix;
  upstream = (import inputs.rust-sel4 {
    nixpkgsPath = inputs.nixpkgs;
    nixpkgsFn = args: import inputs.nixpkgs (args // { localSystem = { inherit system; }; });
  }).overrideNixpkgsArgs (args: args // {
    localSystem = { inherit system; };
    overlays = args.overlays ++ [
      (final: prev: {
        this = prev.this.overrideScope (scope: old: {
          sources = old.sources // { seL4 = inputs.sel4; };
          # Cross builds still link Rust build scripts for the Darwin host.
          buildCratesInLayers = args: old.buildCratesInLayers (args // {
            commonModifications = scope.crateUtils.composeModifications
              (scope.crateUtils.elaborateModifications (args.commonModifications or {}))
              (scope.crateUtils.elaborateModifications {
                modifyDerivation = drv: drv.overrideAttrs (attrs: {
                  depsBuildBuild = (attrs.depsBuildBuild or [])
                    ++ lib.optionals final.stdenv.buildPlatform.isDarwin [ final.buildPackages.libiconv ];
                } // lib.optionalAttrs final.stdenv.buildPlatform.isDarwin {
                  # The pinned nightly's LLVM tools search the wrong relative library directory.
                  DYLD_FALLBACK_LIBRARY_PATH = "${scope.defaultRustToolchain}/lib";
                });
              });
          });
          # GNU and Rust spell the Apple Silicon host triple differently.
          defaultRustTargetTriple =
            if final.stdenv.hostPlatform.isNone then old.defaultRustTargetTriple
            else scope.mkBuiltinRustTargetTriple final.stdenv.hostPlatform.rust.rustcTarget;
          mkDefaultElaborateRustEnvironmentArgs = { rustToolchain }:
            let base = old.mkDefaultElaborateRustEnvironmentArgs { inherit rustToolchain; };
            in base // {
              chooseLinker = args:
                if args.platform.isNone
                then "${rustToolchain}/lib/rustlib/${final.stdenv.buildPlatform.rust.rustcTarget}/bin/rust-lld"
                else base.chooseLinker args;
            };
          # Upstream's internal Fenix import relies on impure builtins.currentSystem.
          defaultUpstreamRustEnvironment = scope.elaborateRustEnvironment (
            scope.mkDefaultElaborateRustEnvironmentArgs {
              rustToolchain = (import inputs.fenix {
                inherit system;
                pkgs = final.buildPackages;
              }).fromToolchainFile {
                file = final.buildPackages.writeText "rust-toolchain.toml" ''
                  [toolchain]
                  channel = "${scope.topLevelRustToolchainFile.attrs.toolchain.channel}"
                  profile = "minimal"
                  components = ["rust-src", "rustfmt", "clippy"]
                '';
                sha256 = "sha256-PDDMZVp1SdCzABXNAy+Unocj2lrQOfZy0EUgu66k520=";
              };
            } // {
              channel = scope.topLevelRustToolchainFile.attrs.toolchain.channel;
              mkCustomTargetPath = _: "${inputs.rust-sel4}/support/targets";
            }
          );
          # This boot test needs no upstream device-model extensions.
          qemuForSeL4 = final.qemu;
        });
      })
    ];
  });
  pkgs = upstream.pkgs.build;
  scope = upstream.pkgs.host.${target.architecture}.none.this;
  world = scope.mkWorld {
    kernelLoaderConfig = {};
    kernelConfig = target.kernelConfig scope.cmakeConfigHelpers;
  };
  # Reuse the official hello example's manifest and locked dependency closure.
  crates = scope.crateUtils.augmentCrates (scope.crates // {
    hello = scope.crates.hello // {
      real = pkgs.linkFarm "boot-root-task-source" {
        "Cargo.toml" = "${scope.crates.hello.real}/Cargo.toml";
        src = ../tests/root-task;
      };
    };
  });
  rootTask = world.mkTask {
    rootCrate = crates.hello;
    release = false;
    runClippy = true;
    lastLayerModifications.extraCargoFlags = [ "--all-features" ];
    lastLayerModifications.modifyDerivation = drv: drv.overrideAttrs (old: {
      postBuild = (old.postBuild or "") + ''
        cargo fmt --check --manifest-path ${old.passthru.workspace}/Cargo.toml -p hello
      '';
    });
  };
  loaderImage = (world.mkSystem { inherit rootTask; }).loaderImage;
  qemuArgs = [
    "${pkgs.qemu}/bin/qemu-system-${target.architecture}"
  ] ++ target.qemuArgs ++ [
    "-kernel" loaderImage
  ];
  qemu = pkgs.writeShellApplication {
    name = "core-qemu";
    text = ''exec ${lib.escapeShellArgs qemuArgs} "$@"'';
  };
  test = pkgs.writeShellApplication {
    name = "core-test";
    text = ''
      exec ${pkgs.python3}/bin/python3 ${../tests/boot.py} \
        --timeout ${toString target.timeoutSeconds} -- ${lib.escapeShellArgs qemuArgs}
    '';
  };
  app = package: { type = "app"; program = lib.getExe package; };
in {
  packages = rec {
    inherit qemu test;
    kernel = world.seL4;
    image = pkgs.runCommand "boot-image" {} ''ln -s ${loaderImage} $out'';
    default = image;
  };
  apps = { qemu = app qemu; test = app test; };
  checks = {
    harness = pkgs.runCommand "boot-harness-tests" {} ''
      export PYTHONDONTWRITEBYTECODE=1
      ${pkgs.python3}/bin/python3 -m unittest discover -s ${../tests} -v
      touch $out
    '';
    boot = pkgs.runCommand "sel4-qemu-boot" {} ''
      ${lib.getExe test} > $out 2>&1 || { cat $out; exit 1; }
      cat $out
    '';
    rust = rootTask;
  };
  shell = pkgs.mkShell {
    packages = [
      pkgs.nix pkgs.qemu pkgs.python3 pkgs.git
      pkgs.this.defaultRustToolchain qemu test
    ];
    RUST_SEL4_SOURCE = inputs.rust-sel4;
    SEL4_SOURCE = inputs.sel4;
    SEL4_PREFIX = world.seL4;
  };
}
