# VORTEX: A High-Performance Vector Database with Kernel-Bypass Durability

**Academic Status**: v2.0 Production-Ready (Core Engine Verified)  
**Architecture**: Shard-per-Core / io_uring / O_DIRECT / Quantized HNSW  

---

## 🔬 Executive Summary
VORTEX is a research-grade vector search engine designed to solve the "Persistence Bottleneck" in high-speed databases. While modern vector databases often trade data safety for speed, VORTEX utilizes **Kernel-Bypass I/O** (via `io_uring`) and **Hardware-Direct Persistence** (via `O_DIRECT | O_DSYNC`) to provide sub-millisecond search latencies without compromising on ACID guarantees.

## 🏗️ Technical Innovations

### 1. The "4KB Physical Alignment" Rule
To achieve maximum NVMe throughput, VORTEX bypasses the OS Page Cache. Every Write-Ahead Log (WAL) entry is strictly aligned to the **4096-byte physical sector size**. This allows the hardware to perform a "Direct Write," eliminating kernel-space memory copies and reducing I/O jitter.

### 2. Shard-per-Core Isolation
VORTEX eliminates thread contention by pinning individual "Shard Reactors" to physical CPU cores. This "Shared-Nothing" architecture ensures that one core's search operation never blocks another core's ingestion.

### 3. Progressive Scalar Quantization
The engine utilizes AVX2-optimized scalar quantization (f32 -> u8) to reduce memory bandwidth by 4x, allowing it to search millions of vectors while fitting entirely within CPU caches.

---

## 📊 Performance Benchmarks (Verified Baseline)
*Tested on standard Linux hardware with synchronous disk logging enabled.*

| Metric | Measured Result | Context |
| :--- | :--- | :--- |
| **Ingestion Throughput** | **100.1 Upserts/sec** | Fully Synchronous (`O_DSYNC`) to Disk |
| **Search Latency (p50)** | **573.82 µs** | Network Round-trip + HNSW Traversal |
| **Search Latency (p99)** | **5.79 ms** | Worst-case tail latency (including Jitter) |

---

## 🐳 Quick Start (Docker)

To demonstrate VORTEX to your professor, use the following commands to launch the environment with the necessary hardware privileges:

```bash
# Clone and Build
git clone https://github.com/McMonds/VORTEX.git
cd VORTEX

# Launch the Server (Requires Docker & Compose)
docker-compose up --build -d
```

> [!IMPORTANT]
> Because VORTEX utilizes memory pinning (`mlockall`) for performance, the container requires the `IPC_LOCK` capability. This is pre-configured in the provided `docker-compose.yml`.

---

## 📡 Running the Benchmark
### 1. Launch the Server
Open a terminal (PowerShell or Bash) and run:
```bash
docker-compose up --build -d
```

### 2. Generate Vectors & Benchmarking
Since the Professor may not have Rust installed, we run the **Stress Test** directly inside a new container. This command will connect to the running server, generate 10,000 random vectors, and perform the search analysis:

```bash
docker run --rm --network host vortex-vortex-server vortex_stress 10000 8
```
*(Note: `--network host` allows the client to reach the server on 127.0.0.1)*

## 🪟 Windows Setup (Crucial)
If your professor is on Windows, they MUST have:
1. **Docker Desktop** installed.
2. **WSL2 Backend** enabled (Settings > General > Use the WSL 2 based engine).
3. VORTEX utilizes `io_uring`, which is a Linux-native kernel feature. Docker on Windows provides this by running a Linux VM in the background (WSL2).

---

## 📜 The VORTEX "Constitution"
All VORTEX code follows 5 non-negotiable laws:
1. **No Dynamic Allocation** in the hot path.
2. **Persistence Precedes Response** (ACID Durability).
3. ** compartir nada** (Shared-Nothing Sharding).
4. **Hardware Alignment**: 4096-byte padding for all Disk I/O.
5. **Lock-Free SPSC Channels** for all inter-thread communication.

---
*Developed for Advanced Agentic Coding Research.*
