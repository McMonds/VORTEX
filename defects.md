# VORTEX Dashboard Defects Tracker

The user has identified 25 defects in the current dashboard implementation. This document tracks the analysis and planned remediation for each.

## 🚨 Critical Architecture Defects

### 1. The Loop Timing Fallacy (Throughput Inflation)
* **Problem**: The code calculates throughput assuming a fixed 100ms loop (`reqs / 0.1`). If rendering slows the loop to 150ms, the math `reqs / 0.1` effectively multiplies the reported throughput by 1.5x (inflation), while the real rate is lower.
* **Analysis**: Standard TUI flaw. `tick_rate` is a *minimum* sleep, not a *guaranteed* interval.
* **Fix Strategy**: Use `Instant::now()` delta (`duration = now - last_tick`). Math: `throughput = delta_reqs / delta_seconds`.

### 2. The Network Blindspot (SoftIRQ Exclusion)
* **Problem**: Metrics ignore `irq` (col 6) and `softirq` (col 7) in `/proc/stat`. High-performance networking (`io_uring`) processes packets in SoftIRQ context (bottom halves).
* **Analysis**: VORTEX is a network database. Ignoring SoftIRQ hides the true cost of packet processing, making the CPU look idle when it's actually thrashing on interrupts.
* **Fix Strategy**: Sum `user + nice + system + irq + softirq` as "Total Work".

### 3. The Queue Parsing Miss (Recv-Q vs Transmit)
* **Problem**: `/proc/net/tcp` col 4 is `tx_queue:rx_queue` (hex). The code might be parsing the whole string or just the first part (TX). Backpressure manifests in the *second* part (RX).
* **Analysis**: We are blind to ingress overload.
* **Fix Strategy**: Split col 4 on `:`, take index 1 (RX), parse from Hex.

### 4. Disk IO Double-Counting
* **Problem**: `/proc/diskstats` lists the root device (`nvme0n1`) and partitions (`nvme0n1p1`). The accumulator sums all lines, effectively doubling (or tripling) the reported bandwidth.
* **Analysis**: Classic stats bug.
* **Fix Strategy**: Filter lines. Ignore `loop*`. Ignore partitions (regex: `sd[a-z]\d+` or `nvme\d+n\d+p\d+`). Only count physical roots.

### 5. The "Silent Death" Masking
* **Problem**: If the server crashes, `fs::read` fails. The code ignores the error (`let _`). The dashboard freezes on the last known "Good" values, misleading the operator.
* **Analysis**: A dashboard's primary job is to report *status*. Freezing is worse than crashing.
* **Fix Strategy**: Handle `NotFound` errors. Switch `server_status` to `CRASHED`. TUI must overlay a big red "OFFLINE" warning.

---

## ⏳ Pending Defects (Waiting for User)

*   Defect 6: [WAITING]
*   ...
*   Defect 25: [WAITING]
