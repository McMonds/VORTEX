# VORTEX Implementation Standard (The Constitution)

This document defines the 15 non-negotiable laws that govern every line of code written for VORTEX. Violation of any single rule disqualifies the engine from "High Performance" status.

---

## SECTION 1: THE MEMORY DOCTRINE
*Guiding Principle: Zero-Entropy Memory Management.*

1. **No Dynamic Allocation in the Hot Path.**
   - **Rule**: Never call `Box::new()`, `Vec::push()`, or `String::from()` inside the Request-Response loop.
   - **Constraint**: Pre-allocate all buffers at startup. Use circular buffers (Ring Buffers) for everything.
2. **Align Everything to 64 Bytes.**
   - **Rule**: Frequent structs must have explicit padding/alignment logic.
   - **Constraint**: Pack structs carefully. Metadata goes in "Structure of Arrays" (SoA), not "Array of Structures".
3. **Pointer Sovereignty (CXL Prep).**
   - **Rule**: Never store 64-bit absolute pointers in WAL or Data files. Use relative 32-bit offsets (Indices).
   - **Reason**: Relative pointers allow Zero-Copy usage immediately after `mmap`.
4. **Respect the Swap.**
   - **Rule**: Attempt to forbid the OS from paging memory.
   - **Adaptive Constraint**: Invoke `mlockall`. If it fails (`EPERM`/`ENOMEM`), emit a clear **CRITICAL WARNING** to stderr and telemetry, then continue in "Jitter-Prone Mode."

---

## SECTION 2: THE CONCURRENCY CONSTITUTION
*Guiding Principle: Shared-Nothing, Lock-Free Silos.*

5. **Mutexes are Illegal.**
   - **Rule**: No `std::sync::Mutex` or `RwLock` in `vortex-core`.
   - **Exception**: Atomic counters or specialized Seqlocks for Telemetry only.
6. **Share Nothing.**
   - **Rule**: Core 0 knows nothing about Core 1's memory.
   - **Constraint**: Cross-core communication must use bounded SPSC lock-free channels.
7. **Cooperative Multitasking.**
   - **Rule**: Long-running loops must manually yield execution every $N$ iterations.
   - **Reason**: No OS scheduler will save a greedy function from halting a shard.

---

## SECTION 3: THE I/O LAW
*Guiding Principle: Direct, Asynchronous, Deterministic Storage.*

8. **Ban `std::fs` and `std::net`.**
   - **Rule**: Standard blocking I/O is prohibited. 
   - **Constraint**: Use `vortex-io` wrapper (interfacing with `io_uring`).
9. **Direct I/O Mandatory.**
   - **Rule**: Handles for the Storage Engine must bypass the OS Page Cache.
   - **Constraint**: Writes must be aligned to physical sector sizes (4096 bytes). 
10. **Persistence Precedes Response.**
    - **Rule**: Never ACK an INSERT until the Completion Queue confirms the disk write.
    - **Exception**: Explicit "Unsafe" mode requests.

---

## SECTION 4: THE SAFETY & SECURITY CODE
*Guiding Principle: Defense-in-Depth and Fault Isolation.*

11. **Zero Trust Ingress.**
    - **Rule**: Validate structurally before parsing logically.
    - **Constraint**: Bytes must pass `rkyv` validation check (alignment/boundaries) before casting.
12. **Defensive Panics.**
    - **Rule**: Use `catch_unwind` to contain shard-level logic bugs.
    - **Constraint**: Reset the volatile state of the affected shard, don't kill the process.
13. **Strict Allocator Quotas.**
    - **Rule**: Track memory usage per IP/Connection.
    - **Constraint**: Disconnect hostile clients before allocation if request frames exceed quotas.

---

## SECTION 5: THE DEVELOPER DISCIPLINE
*Guiding Principle: Rigorous Verification and Metric Purity.*

14. **Test the Math, Not Just the Logic.**
    - **Rule**: All SIMD optimizations must be cross-checked against a "Reference Implementation" test.
15. **Metrics are Atomic.**
   - **Rule**: Collection must compile to a single CPU instruction.
   - **Constraint**: If telemetry is visible in the CPU profiler, it is rejected.

## SECTION 6: ADAPTIVE RESILIENCE
16. **Boot-Time Capability Discovery.**
    - **Rule**: The system must interrogate the kernel and hardware at boot to detect limits (Disk type, ulimit, SIMD).
    - **Constraint**: Configure engine parameters (ring size, alignment, math kernels) dynamically. Never crash because a "top-tier" feature is missing if a stable fallback exists.
