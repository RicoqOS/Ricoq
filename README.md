# Ricoq Operating System (OS)

This is the source code for RicoqOS, an operating system runtime based on the
seL4 microkernel. We aim to implement formal proof inside our code using Verus.

## Architecture

seL4 remains the only privileged kernel on RING0. Everything above it is a
userspace component and may therefore be isolated, restarted, or replaced.

```mermaid
flowchart TB
    APP["Applications"]

    subgraph FRONTENDS["Compatibility frontends"]
        NATIVE["Native Rust API"]
        LIBC["UNIX libc"]
        LINUX["Linux syscall ABI"]
    end

    subgraph USEROS["Userspace OS"]
        UNIX["UNIX semantics"]
        CORE["Async core"]
        ACTOR["Actor & supervision runtime"]
    end

    subgraph PROVIDERS["External providers"]
        VFS["Filesystem"]
        NET["Network"]
        DEV["Devices"]
        RUMP["Rump kernels"]
    end

    SEL4["seL4 microkernel"]
    HW["Hardware"]

    APP --> NATIVE
    APP --> LIBC
    APP --> LINUX

    NATIVE --> CORE
    LIBC --> UNIX
    LINUX --> UNIX

    UNIX --> CORE
    ACTOR --> CORE

    CORE --> PROVIDERS
    RUMP --> VFS
    RUMP --> NET

    CORE --> SEL4
    PROVIDERS --> SEL4
    SEL4 --> HW
```

## Core concepts

* Async-native execution. The runtime is asynchronous by construction.
* UNIX compatibility. The project provides a derived libc replacing syscalls
    with direct calls into the userspace runtime.
* Actor model. Isolation and fault tolerance inspired by OTP.
