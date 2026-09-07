rec {
  architecture = "aarch64";
  platform = "qemu-arm-virt";
  cpu = "cortex-a57";
  memoryMiB = 1024;
  cores = 1;
  timeoutSeconds = 120;

  kernelConfig = { mkString, on, off, ... }: {
    KernelArch = mkString "arm";
    KernelSel4Arch = mkString architecture;
    KernelPlatform = mkString platform;
    ARM_CPU = mkString cpu;
    QEMU_MEMORY = mkString (toString memoryMiB);
    KernelMaxNumNodes = mkString (toString cores);
    KernelVerificationBuild = off;
    KernelDebugBuild = on;
    KernelPrinting = on;
    KernelArmHypervisorSupport = on;
  };

  qemuArgs = [
    "-machine" "virt,virtualization=on"
    "-accel" "tcg,thread=single"
    "-cpu" cpu
    "-smp" (toString cores)
    "-m" (toString memoryMiB)
    "-display" "none"
    "-monitor" "none"
    "-serial" "stdio"
    "-nic" "none"
    "-no-reboot"
  ];
}
