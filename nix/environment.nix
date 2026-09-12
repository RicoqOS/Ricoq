{
  inputs,
  system,
}: let
  inherit (inputs.nixpkgs) lib;
  target = import ./target.nix;

  upstream =
    (import inputs.rust-sel4 {
      nixpkgsPath = inputs.nixpkgs;
      nixpkgsFn = args: import inputs.nixpkgs (args // {localSystem = {inherit system;};});
    }).withOverlays [
      (final: prev: {
        this = prev.this.overrideScope (scope: old: {
          sources = old.sources // {seL4 = inputs.sel4;};

          buildCratesInLayers = args:
            old.buildCratesInLayers (args
              // {
                commonModifications =
                  scope.crateUtils.composeModifications
                  (scope.crateUtils.elaborateModifications (args.commonModifications or {}))
                  (scope.crateUtils.elaborateModifications {
                    modifyDerivation = drv:
                      drv.overrideAttrs (attrs:
                        {
                          depsBuildBuild =
                            (attrs.depsBuildBuild or [])
                            ++ lib.optionals final.stdenv.buildPlatform.isDarwin [final.buildPackages.libiconv];
                        }
                        // lib.optionalAttrs final.stdenv.buildPlatform.isDarwin {
                          DYLD_FALLBACK_LIBRARY_PATH = "${scope.defaultRustToolchain}/lib";
                        });
                  });
              });

          defaultRustTargetTriple =
            if final.stdenv.hostPlatform.isNone
            then old.defaultRustTargetTriple
            else scope.mkBuiltinRustTargetTriple final.stdenv.hostPlatform.rust.cargoShortTarget;

          mkDefaultElaborateRustEnvironmentArgs = {rustToolchain}: let
            base = old.mkDefaultElaborateRustEnvironmentArgs {inherit rustToolchain;};
          in
            base
            // {
              chooseLinker = args:
                if args.platform.isNone
                then "${rustToolchain}/lib/rustlib/${final.stdenv.buildPlatform.rust.cargoShortTarget}/bin/rust-lld"
                else base.chooseLinker args;
            };

          defaultUpstreamRustEnvironment = scope.elaborateRustEnvironment (
            scope.mkDefaultElaborateRustEnvironmentArgs {
              rustToolchain =
                (import inputs.fenix {
                  inherit system;
                  pkgs = final.buildPackages;
                }).fromToolchainFile {
                  file = ../rust-toolchain.toml;
                  sha256 = "sha256-PDDMZVp1SdCzABXNAy+Unocj2lrQOfZy0EUgu66k520=";
                };
            }
            // {
              channel = scope.topLevelRustToolchainFile.attrs.toolchain.channel;
              mkCustomTargetPath = _: "${inputs.rust-sel4}/support/targets";
            }
          );
          qemuForSeL4 = final.buildPackages.qemu;
        });
      })
    ];

  pkgs = upstream.pkgs.build;
  scope = upstream.pkgs.host.${target.architecture}.none.this;
  world = scope.mkWorld {
    kernelLoaderConfig = {};
    kernelConfig = target.kernelConfig scope.cmakeConfigHelpers;
  };

  rustTarget = scope.defaultRustTargetTriple;
  rustTargetName = rustTarget.name;
  rustTargetEnv = scope.crateUtils.toUpperWithUnderscores rustTargetName;
  hostRustTarget = pkgs.stdenv.hostPlatform.rust.cargoShortTarget;
  toolchain = pkgs.this.defaultRustToolchain;

  sysroot = scope.buildSysroot {
    std = false;
    profile = "release";
  };

  cargoEnvironment =
    {
      CARGO_BUILD_TARGET = rustTargetName;
      SEL4_PREFIX = world.seL4;
      RUST_TARGET_PATH = "${inputs.rust-sel4}/support/targets";
      LIBCLANG_PATH = scope.libclangPath;
      "CARGO_TARGET_${rustTargetEnv}_RUSTFLAGS" = "-Zunstable-options --sysroot ${sysroot}";
    }
    // lib.optionalAttrs pkgs.stdenv.isDarwin {
      DYLD_FALLBACK_LIBRARY_PATH = "${toolchain}/lib";
    };

  cargoInputs = [toolchain pkgs.stdenv.cc] ++ lib.optionals pkgs.stdenv.isDarwin [pkgs.libiconv];

  vendor = scope.vendorLockfile {lockfile = ../Cargo.lock;};
  vendorConfig = scope.crateUtils.toTOMLFile "cargo-vendor.toml" vendor.configFragment;
  substrateSel4 = ../crates/substrate;

  rustChecks = pkgs.runCommand "substrate-cargo-checks" (cargoEnvironment // {nativeBuildInputs = cargoInputs;}) ''
    cp -R ${../.} source
    chmod -R u+w source
    cd source
    export CARGO_HOME="$TMPDIR/cargo-home"
    mkdir -p "$CARGO_HOME"
    cp ${vendorConfig} "$CARGO_HOME/config.toml"
    export CARGO_NET_OFFLINE=true

    cargo fmt --check
    cargo clippy --locked --workspace --all-targets --all-features --target ${rustTargetName} -- -D warnings
    rustc --edition 2024 --test crates/substrate/src/free_slots.rs --target ${hostRustTarget} -o free-slots-tests
    ./free-slots-tests
    rustc --edition 2024 --test crates/substrate/src/task.rs --target ${hostRustTarget} -o task-tests
    ./task-tests
    rustc --edition 2024 --test crates/substrate/src/fault.rs --target ${hostRustTarget} -o fault-tests
    ./fault-tests
    rustc --edition 2024 --test crates/substrate/src/ipc_state.rs --target ${hostRustTarget} -o ipc-state-tests
    ./ipc-state-tests
    cargo build --locked -p substrate-sel4 --bin substrate-sel4 --target ${rustTargetName}
    ${pkgs.python3}/bin/python3 crates/substrate/tests/image.py --production target/${rustTargetName}/debug/substrate-sel4.elf
    cargo test --locked -p substrate-sel4 --test substrate-integration --no-run --target ${rustTargetName} --message-format=json > test-build.json
    testElf="$(${pkgs.python3}/bin/python3 -c 'import json; print(next(m["executable"] for line in open("test-build.json") if (m := json.loads(line)).get("executable") and m["target"]["name"] == "substrate-integration"))')"
    ${pkgs.python3}/bin/python3 crates/substrate/tests/image.py "$testElf"

    mkdir -p "$out/bin"
    cp target/${rustTargetName}/debug/substrate-sel4.elf "$out/bin/"
    cp "$testElf" "$out/bin/substrate-integration.elf"
  '';

  rootTask = {elf = "${rustChecks}/bin/substrate-sel4.elf";};
  loaderImage = (world.mkSystem {inherit rootTask;}).loaderImage;
  testImage = (world.mkSystem {rootTask.elf = "${rustChecks}/bin/substrate-integration.elf";}).loaderImage;

  qemuArgs = ["${pkgs.qemu}/bin/qemu-system-${target.architecture}"] ++ target.qemuArgs ++ ["-kernel" loaderImage];
  qemuCommand = lib.escapeShellArgs qemuArgs;
  testCommand = lib.escapeShellArgs (["${pkgs.qemu}/bin/qemu-system-${target.architecture}"] ++ target.qemuArgs ++ ["-kernel" testImage]);

  qemu = pkgs.writeShellApplication {
    name = "core-qemu";
    text = "exec ${qemuCommand} \"$@\"";
  };

  test = pkgs.writeShellApplication {
    name = "core-test";
    text = ''
      exec ${pkgs.python3}/bin/python3 ${substrateSel4}/tests/boot.py \
        --timeout ${toString target.timeoutSeconds} \
        -- ${testCommand}
    '';
  };

  app = package: {
    type = "app";
    program = lib.getExe package;
  };
in {
  formatter = pkgs.alejandra;
  packages = rec {
    inherit qemu test;
    kernel = world.seL4;
    image = pkgs.runCommand "boot-image" {} ''ln -s ${loaderImage} $out'';
    default = image;
  };
  apps = {
    qemu = app qemu;
    test = app test;
  };
  checks = {
    nix-format = pkgs.runCommand "nix-format-check" {} ''
      ${lib.getExe pkgs.alejandra} --check ${../.}
      touch "$out"
    '';
    harness = pkgs.runCommand "boot-harness-tests" {} ''
      export PYTHONDONTWRITEBYTECODE=1
      cd ${substrateSel4}/tests
      ${pkgs.python3}/bin/python3 -m unittest discover -v
      touch "$out"
    '';
    boot = pkgs.runCommand "sel4-qemu-boot" {} ''
      ${lib.getExe test} > "$out" 2>&1 || { cat "$out"; exit 1; }
      cat "$out"
    '';
    rust = rustChecks;
  };
  shell = pkgs.mkShell (cargoEnvironment
    // {
      packages =
        cargoInputs
        ++ [
          pkgs.nix
          pkgs.qemu
          pkgs.python3
          pkgs.git
          pkgs.alejandra
        ];
      RUST_SEL4_SOURCE = inputs.rust-sel4;
      SEL4_SOURCE = inputs.sel4;
    });
}
