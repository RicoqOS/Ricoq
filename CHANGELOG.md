# Changelog

## 0.0.1

### Added

- Nix flake setup to build and boot an unpatched seL4 16.0.0 kernel.
- AArch64 `qemu-arm-virt` target (Cortex-A57, 1 GiB RAM) using emulation.
- Unified dev shell and build rules for Linux and macOS (x86-64 & AArch64).
- QEMU runner, boot test harness, and `nix flake check` integration for CI.
