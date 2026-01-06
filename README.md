# VORTEX: High-Performance Distributed Vector Database

VORTEX is a next-generation, high-performance distributed vector search engine designed for microsecond-level predictability and massive throughput. It utilizes a **shared-nothing, shard-per-core** architecture with **kernel-bypass I/O** and **zero-copy** data paths to saturate modern NVMe and 100/400GbE hardware.

## 🚀 Performance Baseline
Derived from **Vector Engine v2.1** baseline on Ryzen 5 (6C/12T):
- **Throughput**: ~204K QPS (1M vectors, ef=32)
- **Mean Latency**: ~18.0 µs
- **VORTEX Target**: > 1.8M OPS with < 10µs mean latency.

## 🏗️ Architectural Pillars
- **Kernel Bypass**: Utilizing `io_uring` for asynchronous, zero-copy networking and storage I/O.
- **Shard-Per-Core**: 1:1 static pinning of execution threads to physical CPU cores to eliminate context switching.
- **Memory Sovereignty**: Pre-allocated HugePage arenas with `mlockall` to prevent swap-induced jitter.
- **Adaptive Resilience**: Real-time hardware interrogation (SIMD, NUMA, Sector Size) with graceful fallback for sub-optimal environments.
- **Zero-Trust Security**: In-place mTLS (rustls) and Biscuit-based capability authorization.

## 🛠️ Project Structure (Skeleton Phase)
- `vortex-io`: Platform-specific PAL, `io_uring` RingDriver, and Hardware Topology mapping.
- `vortex-core`: Shard execution loop, LSM-Tree storage, and HNSW graph index.
- `vortex-rpc`: Zero-copy binary protocol (VBP) definitions via `rkyv`.
- `vortex-server`: The orchestration binary and control plane.

## 🚦 Current Status: Milestone 1 Complete
- [x] **Platform Skeleton**: Core pinning and memory locking logic.
- [x] **Hardware Interrogation**: NUMA and physical core detection.
- [x] **Adaptive Resilience**: Graceful `mlockall` fallback and disk sector size discovery.
- [x] **io_uring Driver**: Initialized async submission/completion queues.

## 📦 Getting Started

### Prerequisites
- **Arch Linux** (Recommended) or Linux Kernel 6.1+.
- `libhwloc` (Install via `sudo pacman -S hwloc`).

### Running the Server
```bash
RUST_LOG=info cargo run -p vortex-server
```

## 📜 Constitution
VORTEX is governed by a strict **Implementation Standard** (see `standard.md`):
1. No Dynamic Allocation in the Hot Path.
2. Align Everything to 64 Bytes.
3. Mutexes are Illegal.
4. Share Nothing.
5. Persistence Precedes Response.

---
*VORTEX: Bypassing the OS, Saturating the Metal.*
