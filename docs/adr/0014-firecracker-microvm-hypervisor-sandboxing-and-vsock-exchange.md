# ADR 14: Firecracker MicroVM Hypervisor Sandboxing and Vsock Exchange

* **Status**: Accepted
* **Decided on**: 2026-07-03

## Context

While our `DockerBackend` (ADR 6) successfully isolates the project workspace filesystem and shell environment, the container shares the host operating system's Linux kernel. Running fully untrusted, autonomous agentic loops or compiling unknown third-party source packages on a shared kernel exposes CADE to potential kernel privilege-escalation exploits or container escape vectors.

To provide production-grade, hardware-virtualized sandboxing, we need to design an alternative backend that isolates execution at the hypervisor level.

## Decision

We decided to implement a hardware-virtualized **`MicroVmBackend`** powered by **AWS Firecracker** with an isolated **Vsock Communication Channel**:

### 1. Hypervisor-Level `vsock` Control Channel
CADE will communicate with the guest MicroVM strictly over virtual sockets (`vsock`) at the hypervisor bus level.
* A lightweight, dependency-free Rust binary (`cade-guest-daemon`) is pre-baked inside the guest rootfs and boots automatically.
* The host `CadeAgent` opens a `vsock` connection directly into the hypervisor, exchanging structured JSON-RPC payloads (e.g. executing commands or checking files).
* This eliminates the need for host-side virtual network interfaces (`tap`, bridges, or open ports), keeping the communication network-isolated.

### 2. Docker-to-Ext4 Loopback Conversion (`rootfs`)
To build the guest ext4 root filesystem images cleanly and flexibly:
* CADE includes a helper script `cade-vm-builder` that pulls standard, development-oriented Docker images (e.g. `clux/muslrust` or `ubuntu:22.04`).
* The script exports the docker container's root filesystem and writes the raw bytes directly into an ext4 loopback image file.
* This leverages Docker's massive ecosystem, allowing users to reuse their standard `Dockerfile` definitions to compile and package MicroVM disks.

### 3. Hypervisor-Isolated Vsock Tarball Exchange
To preserve absolute hardware boundary isolation during project file synchronization:
* Before booting the MicroVM, the host `CadeAgent` packages the local project directory (respecting `.gitignore` exclusions) into an in-memory tarball.
* The host streams this tarball over the `vsock` channel to the guest, where the daemon extracts it under `/workspace`.
* When the subagents complete their tasks, the guest daemon packages `/workspace` back into a tarball and streams it back over `vsock`, allowing the host to cleanly merge the files.
* No directories or volume mounts are shared between host and guest, completely blocking any directory-traversal exploits or guest escape vectors.

## Consequences

### Positive (Pros)
* **Hardware-Level Isolation**: Any kernel panics, infinite compilation loops, or exploits are 100% trapped inside the virtual machine's guest kernel, with zero risk to the host.
* **Frictionless Portability**: CADE can simulate different OS distributions, library packages, and architectures independently of your host OS.
* **Network-Isolated Security**: Communication over `vsock` requires no open network interfaces, making the sandbox fully secure.

### Negative (Cons)
* **Initial Boot Overhead**: Booting a fresh MicroVM adds 5-10ms of latency (though still significantly faster than standard full VMs).
* **Storage Overhead**: Generating and storing 1-2GB loopback ext4 disk images requires additional local storage space.
