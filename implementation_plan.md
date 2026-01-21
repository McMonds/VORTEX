# VORTEX Detailed Implementation Plan

This plan breaks down the VORTEX project into manageable milestones based on the locked **Logical Blueprint (Technical Atlas)** and the **VORTEX Implementation Standard (The Constitution)**. Every line of code must adhere to the 15 non-negotiable laws define in the standard.

---

## Performance Baseline: Vector Engine v2.1
VORTEX is designed to scale the existing performance of Vector Engine v2.1.

| Metric | Baseline (v2.1) | VORTEX Target (p99) |
| :--- | :--- | :--- |
| **Throughput (QPS)** | ~204K (1M vectors, ef=32) | > 1.8M OPS |
| **Mean Latency** | ~18.0 µs | < 10 µs |
| **P99 Tail** | ~33 µs | < 50 µs (under saturation) |
| **Efficiency** | ~22MB RSS (10k vectors) | < 4GB per 1M vectors |

---

## Milestone 1: The Platform & Skeleton (vortex-io)
**Objective**: Establish the "Noise-Free Execution Chamber" and basic hardware telemetry.

- **Tasks**:
    - [x] Skeleton setup: `vortex-io`, `vortex-core`, `vortex-server`.
    - [x] **Adaptive Hardware Discovery**: Logic to detect physical core count, NUMA nodes, and Disk physical sector size.
    - [x] **Resilient Memory Locking**: Attempt `mlockall` but handle `ENOMEM` gracefully with logging.
    - [x] Implement `ThreadMapping`: Static pinning of LEUs (Logical Execution Units).
    - [x] Initialize `io_uring` RingDriver: Setup Submission/Completion queues.
- **Verification**:
    - CLI tools to verify thread affinity (core pinning) matches the intended LEU mapping.
    - Benchmarking `io_uring` NOP submissions to verify zero context-switch baseline.

---

## Milestone 2: Wire Protocol & Memory Sovereignty (vortex-rpc)
**Objective**: Implement the VBP (VORTEX Binary Protocol) and the zero-copy buffer lifecycle.

- **Tasks**:
    - [x] Define VBP Request/Response frames with `rkyv`.
    - [x] Implement the `BufferPool`: Pre-allocated memory rings with handover logic.
    - [ ] Security Gatekeeper: Integreate `rustls` (mTLS) and `Biscuit` (Auth) on raw buffer slices.
    - [x] Implement `LayoutValidator`: Strict mathematical check of### [Dashboard Core] (main.rs)
- **CLI Expansion**: Implement `-p/--port`, `-d/--dir`, `-c/--capacity`, and `--shards`.
- **Log parser**: Add capture for `Search: N ops, T us, D dists` and `Health: I ms ingress, F ms flush` pulses.
- **Uptime Tracking**: Calculate run duration in Minutes:Seconds.
- **Total Ops**: Global cumulative counter for successful ACKs.

### [Orchestration] (lifecycle.rs)
- **Arg Forwarding**: Update `spawn_server` to accurately forward all dashboard CLI parameters to the `vortex-server` binary.

### [Forensics] (metrics.rs)
- **Network Efficiency**: Parse `/proc/net/dev` for raw TX/RX Mbps. Calculate "Packet Overhead Ratio" (VBP Payload / Raw Wire Bytes).
- **Dynamic Port**: construction of `target_port_suffix` using the hex constraint.
- **CPU Breakdown**: Implement parsing for `User%`, `System%`, `SoftIRQ%`.
- **Context Switches**: Parse voluntary/involuntary switches from `/proc/stat`.

### [Reactor Core] (reactor.rs)
- **Compute Layer**: Track `tick_search_micros`, `tick_search_ops`, `tick_nodes_inspected`.
- **Contention Profiling**: Measure `time_ingress` (logic) vs `time_flush` (persistence).
- **Aggregated Logging**: Emit 1Hz pulse: `PULSE [Search] ops={N} time={T}us dist={D} | [Health] ingress={I}ms flush={F}ms`.

### [TUI Layout] (tui.rs)
- **Section A: Mission Header**: Uptime, Build Mode, Instant/Peak Throughput, Total Ops.
- **Section B: Engine Dynamics**: 
    *   Batch Saturation Bar & Avg Batch Size.
    *   Flush Ratios (Full vs EOT).
    *   **WAF**: (Physical Disk Bytes / Logical Ingress Bytes).
    *   **Search Stats**: Dedicated QPS & Latency Histogram (P50/P99).
- **Section C: Hardware Stress**: 
    *   Core Util Bars & Core Balance (StdDev).
    *   Context Switch Rate.
    *   CPU Breakdown (User/Sys/SoftIRQ).
    *   **Shard Health**: Opcode Ratio (W/R), Cycle Starvation (Ingress vs Flush).
    *   **Warning**: "READ LATENCY RISK" if Flush Time > 20ms.
- **Section D: Diagnostics**: 
    *   Rx-Queue Depth & TCP Collisions/Drops.
    *   **Wire Speed**: Physical Mbps (TX/RX).
    *   **Efficiency**: Packet Overhead Ratio.

### [Benchmark Utility] (stress_test.rs)
- **Mixed Mode**: Implement `--mode mixed` (8 Writers / 8 Readers).
- **Precise Histograms**: Collect `Vec<Duration>` per thread, merge, and sort for the 4-block receipt (P50, P99, Max/Jitter).

---

## Milestone 3: The Shard Reactor Loop (vortex-core)
**Objective**: Build the single-threaded, non-blocking execution reactor.

- **Tasks**:
    - [x] Develop the `ShardCoordinator` Event Loop.
    - [x] Implement the Reactor: Collaborative multitasking via `async/await` state machines.
    - [x] **Backpressure Valve**: Implement `SQE_Depth` monitoring and `THROTTLED` state transition.
    - [x] Internal Messaging: SPSC Lock-free channels for inter-shard mis-routing handovers.
- **Verification**:
    - Latency jitter tests on the polling loop under high artificial load.
    - Verify TCP window throttling when `SQE_Depth > High_Water_Mark`.

---

## Milestone 4: Storage Engine & Durability (vortex-storage)
**Objective**: Implement the LSM-Tree and WAL with crash recovery guarantees.

- **Tasks**:
    - [x] **BOOT_REPLAY**: Blocking WAL replay logic and `MANIFEST` checkpoint management.
    - [x] WAL Implementation: `O_DIRECT | O_DSYNC` appends with `IOSQE_IO_LINK` ordering.
    - [x] MemTable: Single-author Skiplist/RB-Tree (Lock-free).
    - [x] SSTable: 4KB block-aligned storage with CRC32C integrity checksums.
- **Verification**:
    - Torn-write recovery: Hard-killing the process during WAL writes and verifying successful truncation/replay.
    - Corruption scrubbing: Manually flipping bits in SSTables to trigger CRC failures and Raft repair (simulated).

---

## Milestone 5: The Vector Search Index (Integration)
**Objective**: Integrate Vector Engine v2.1 and optimize via SIMD/Kernel-Bypass.

- **Tasks**:
    - [x] **Engine Porting**: Wrap existing Vector Engine v2.1 logic as a `vortex-core` component.
    - [x] Distance Metrics: Upgrade existing math to AVX-512 for Ryzen/EPYC performance.
    - [x] HNSW Graph: Shift from standard pointers to CSR-style 32-bit relative pointers.
    - [ ] **Compaction Hook**: Implement the side-car index builder during LSM merges.
- **Verification**:
    - Regression Testing: Ensure VORTEX matches or exceeds v2.1 throughput on identical hardware.
    - Recall benchmarks on SIFT1M using the integrated engine.

---

## Milestone 6: Distribution & Observability (The Grid)
**Objective**: Scaling out with Raft and enabling deep introspection.

- **Tasks**:
    - [ ] **Raft Consistency**: One Raft group per VNode; multi-AZ placement policy.
    - [x] Consistent Hashing: Ketama ring for deterministic request routing.
    - [x] Telemetry Sidecar: Lock-free per-shard registries and Seqlock aggregation.
    - [x] **Tail-Based Tracing**: 1,000-request circular buffers for error/latency dumping.
- **Verification**:
    - Jepsen linearizability tests under network partition and clock skew.
    - Profiling with `eBPF` to verify zero observer effect on high-performance cores.
