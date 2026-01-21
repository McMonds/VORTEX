use std::collections::HashSet;
use std::fs;
use std::time::Instant;
use anyhow::Result;

// =================================================================================
// 1. Raw Snapshot (Pure Data Layer)
// Holds raw u64 counters. Zero logic. Zero rates. Zero floats.
// Source: /proc/stat, /proc/diskstats, /proc/net/*
// =================================================================================
#[derive(Debug, Clone, Default)]
pub struct RawSnapshot {
    pub timestamp: Option<Instant>, // Added Option for clear "Before Start" state
    
    // CPU: Absolute Ticks (USER_HZ)
    pub cpu_total_ticks: Vec<u64>, 
    pub cpu_work_ticks: Vec<u64>, 
    pub cpu_user_ticks: Vec<u64>,
    pub cpu_system_ticks: Vec<u64>,
    pub cpu_softirq_ticks: Vec<u64>,

    // Disk: Absolute Sectors
    pub disk_sectors_written: u64,

    // Net: Absolute Packets/Bytes
    pub net_rx_queue: u64, // Instantaneous
    pub net_rx_bytes: u64,
    pub net_tx_bytes: u64,
    pub net_prune_called: u64, // Cumulative Counter
    
    // Mem: Absolute KB
    pub memory_rss_kb: u64,
    pub context_switches: u64,
}

// =================================================================================
// 2. Metrics Snapshot (Presentation Layer)
// The calculated rates (Delta / Time) ready for TUI.
// =================================================================================
#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    pub timestamp: Instant,
    
    // Hardware Rates
    pub cpu_usage_pct: Vec<f64>, // Index 0 = Global, 1..N = Cores
    pub sys_efficiency_pct: f64, // (Sys + IRQ + SoftIRQ) / Total Work
    pub rss_mem_mb: f64,
    
    // IO Rates
    pub disk_write_mb_s: f64,
    pub net_rx_backlog: u64,
    pub net_prunes_per_sec: f64,
    
    // Foreman Sub-Layer Metrics
    pub net_tx_mbps: f64,
    pub net_rx_mbps: f64,
    pub net_efficiency_ratio: f64,
    pub cpu_user_pct: Vec<f64>,
    pub cpu_system_pct: Vec<f64>,
    pub cpu_softirq_pct: Vec<f64>,
    pub context_switches_per_sec: f64,
}

impl Default for MetricsSnapshot {
    fn default() -> Self {
        Self {
            timestamp: Instant::now(),
            cpu_usage_pct: vec![],
            sys_efficiency_pct: 0.0,
            rss_mem_mb: 0.0,
            disk_write_mb_s: 0.0,
            net_prunes_per_sec: 0.0,
            net_rx_backlog: 0,
            net_tx_mbps: 0.0,
            net_rx_mbps: 0.0,
            net_efficiency_ratio: 0.0,
            cpu_user_pct: vec![],
            cpu_system_pct: vec![],
            cpu_softirq_pct: vec![],
            context_switches_per_sec: 0.0,
        }
    }
}

// =================================================================================
// 3. System Sampler (The Logic Layer)
// Handles Sampling, Deltas, Normalization, and Safe Math.
// =================================================================================
pub struct SystemSampler {
    // Static Hardware Constants
    sector_size: u64,
    valid_disks: HashSet<String>,
    target_port_suffix: String,
    
    // State
    prev_snapshot: RawSnapshot,
    server_pid: Option<u32>,
}

impl SystemSampler {
    pub fn new(server_pid: Option<u32>, port: u16) -> Self {
        let sector_size = Self::detect_sector_size();
        let valid_disks = Self::scan_physical_disks();
        
        // Constraint 2: The Port Hex Hex
        let target_port_suffix = format!(":{:04X}", port);

        Self {
            sector_size,
            valid_disks,
            target_port_suffix,
            prev_snapshot: RawSnapshot::default(),
            server_pid,
        }
    }

    // Constraint 1: The Disk Deduplication (Verification)
    // Only trust devices that DO NOT have a specific partition signature
    fn scan_physical_disks() -> HashSet<String> {
        let mut valid = HashSet::new();
        if let Ok(entries) = fs::read_dir("/sys/block") {
            for entry in entries.flatten() {
                if let Ok(name) = entry.file_name().into_string() {
                    // Skip loopback and ram disks
                    if name.starts_with("loop") || name.starts_with("ram") { continue; }
                    
                    // We want: sda, nvme, and dm- (LVM/Mapper)
                    // Partitions usually have a 'partition' file in sysfs, roots don't.
                    let part_path = entry.path().join("partition");
                    if !part_path.exists() {
                        valid.insert(name);
                    }
                }
            }
        }
        valid
    }

    fn detect_sector_size() -> u64 {
        // Try reading nvme0n1 first, then sda... fallback to 512.
        if let Ok(entries) = fs::read_dir("/sys/block") {
             for entry in entries.flatten() {
                 let name = entry.file_name().into_string().unwrap_or_default();
                 if name.starts_with("loop") { continue; }
                 
                 let queue_path = entry.path().join("queue/hw_sector_size");
                 if let Ok(s) = fs::read_to_string(queue_path) {
                     if let Ok(v) = s.trim().parse::<u64>() {
                         return v;
                     }
                 }
             }
        }
        512 // Legacy Fallback
    }

    pub fn capture(&mut self) -> Result<MetricsSnapshot> {
        let now = Instant::now();
        let mut raw = RawSnapshot {
            timestamp: Some(now),
            cpu_total_ticks: vec![],
            cpu_work_ticks: vec![],
            cpu_user_ticks: vec![],
            cpu_system_ticks: vec![],
            cpu_softirq_ticks: vec![],
            disk_sectors_written: 0,
            net_rx_queue: 0,
            net_tx_bytes: 0,
            net_rx_bytes: 0,
            net_prune_called: 0,
            memory_rss_kb: 0,
            context_switches: 0,
        };

        // --- 1. Parse /proc/stat (CPU & Context Switches) ---
        let stat = fs::read_to_string("/proc/stat")?;
        for line in stat.lines() {
            if line.starts_with("cpu") && line.as_bytes().get(3).map_or(false, |&b| b.is_ascii_digit()) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() < 8 { continue; }

                let user: u64 = parts[1].parse().unwrap_or(0);
                let nice: u64 = parts[2].parse().unwrap_or(0);
                let system: u64 = parts[3].parse().unwrap_or(0);
                let idle: u64 = parts[4].parse().unwrap_or(0);
                let iowait: u64 = parts[5].parse().unwrap_or(0);
                let irq: u64 = parts[6].parse().unwrap_or(0);
                let softirq: u64 = parts[7].parse().unwrap_or(0);

                let work = user + nice + system + irq + softirq;
                let total = work + idle + iowait;

                raw.cpu_work_ticks.push(work);
                raw.cpu_total_ticks.push(total);
                raw.cpu_user_ticks.push(user + nice);
                raw.cpu_system_ticks.push(system + irq);
                raw.cpu_softirq_ticks.push(softirq);
            } else if line.starts_with("ctxt ") {
                raw.context_switches = line[5..].trim().parse().unwrap_or(0);
            }
        }
        
        // --- 2. Parse /proc/diskstats (Disk) ---
        // Constraint 1: Filter using `valid_disks` set
        let diskwrapper = fs::read_to_string("/proc/diskstats")?;
        for line in diskwrapper.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() > 13 {
                let dev_name = parts[2];
                if self.valid_disks.contains(dev_name) {
                    // Column 10 (index 9) = sectors written
                    let written: u64 = parts[9].parse().unwrap_or(0);
                    raw.disk_sectors_written += written;
                }
            }
        }

        // --- 3. Parse /proc/net/tcp (RxQueue) ---
        if let Ok(tcp) = fs::read_to_string("/proc/net/tcp") {
            for line in tcp.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() > 4 && parts[1].ends_with(&self.target_port_suffix) {
                     // Column 4: tx:rx (hex). Split on ':' -> Index 1 is RX
                     if let Some(col) = parts.get(4) {
                         let queues: Vec<&str> = col.split(':').collect();
                         if queues.len() == 2 {
                             // Sum across all shards/connections
                             let q = u64::from_str_radix(queues[1], 16).unwrap_or(0);
                             raw.net_rx_queue += q;
                         }
                     }
                }
            }
        }
        
        // --- 4. Parse /proc/net/dev (Mbps) ---
        if let Ok(dev) = fs::read_to_string("/proc/net/dev") {
            for line in dev.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() > 9 {
                    if parts[0].starts_with("lo:") { continue; }
                    let rx: u64 = parts[1].parse().unwrap_or(0);
                    let tx: u64 = parts[9].parse().unwrap_or(0);
                    raw.net_rx_bytes += rx;
                    raw.net_tx_bytes += tx;
                }
            }
        }
        
        // --- 4. Parse /proc/net/netstat (PruneCalled) ---
        // Defect 15: The Recv-Q Snapshot Lie
        if let Ok(netstat) = fs::read_to_string("/proc/net/netstat") {
             // Need "TcpExt:" header then values
             let lines: Vec<&str> = netstat.lines().collect();
             for i in (0..lines.len()).step_by(2) {
                 if lines[i].starts_with("TcpExt:") {
                     let headers: Vec<&str> = lines[i].split_whitespace().collect();
                     let values: Vec<&str> = lines[i+1].split_whitespace().collect();
                     
                     // Find "PruneCalled" index
                     if let Some(idx) = headers.iter().position(|&x| x == "PruneCalled") {
                         if let Some(val) = values.get(idx) {
                             raw.net_prune_called = val.parse().unwrap_or(0);
                         }
                     }
                 }
             }
        }
        
        // --- 5. Parse RSS (Server Check) ---
        // --- 5. Parse RSS (Server Check) ---
        if let Some(pid) = self.server_pid {
            if let Ok(status) = fs::read_to_string(format!("/proc/{}/status", pid)) {
                for line in status.lines() {
                     if line.starts_with("VmRSS:") {
                         if let Some(val_str) = line.split_whitespace().nth(1) {
                             raw.memory_rss_kb = val_str.parse().unwrap_or(0);
                         }
                     }
                }
            } else {
                 // Process missing ? Main loop might handle via channel event. 
                 // We can signal failure implicitly by 0 RSS or let main handle it.
            }
        }
        
        // CALCULATE DELTAS
        let mut metrics = MetricsSnapshot::default();
        if let Some(prev_time) = self.prev_snapshot.timestamp {
             let delta_t = now.duration_since(prev_time).as_secs_f64();
             
             if delta_t > 0.0 {
                  // CPU Breakdown
                  for (idx, &curr_total) in raw.cpu_total_ticks.iter().enumerate() {
                      if let Some(&prev_total) = self.prev_snapshot.cpu_total_ticks.get(idx) {
                          let d_total = s_sub(curr_total, prev_total).max(1);
                          
                          let usage = (s_sub(raw.cpu_work_ticks[idx], self.prev_snapshot.cpu_work_ticks[idx]) as f64 / d_total as f64) * 100.0;
                          let user = (s_sub(raw.cpu_user_ticks[idx], self.prev_snapshot.cpu_user_ticks[idx]) as f64 / d_total as f64) * 100.0;
                          let sys = (s_sub(raw.cpu_system_ticks[idx], self.prev_snapshot.cpu_system_ticks[idx]) as f64 / d_total as f64) * 100.0;
                          let soft = (s_sub(raw.cpu_softirq_ticks[idx], self.prev_snapshot.cpu_softirq_ticks[idx]) as f64 / d_total as f64) * 100.0;
                          
                          metrics.cpu_usage_pct.push(usage);
                          metrics.cpu_user_pct.push(user);
                          metrics.cpu_system_pct.push(sys);
                          metrics.cpu_softirq_pct.push(soft);
                      }
                  }
                  
                  // IO
                  let d_sectors = s_sub(raw.disk_sectors_written, self.prev_snapshot.disk_sectors_written);
                  let bytes = d_sectors * self.sector_size;
                  metrics.disk_write_mb_s = (bytes as f64 / 1_048_576.0) / delta_t;
                  
                  // Net Flow
                  let d_rx = s_sub(raw.net_rx_bytes, self.prev_snapshot.net_rx_bytes);
                  let d_tx = s_sub(raw.net_tx_bytes, self.prev_snapshot.net_tx_bytes);
                  metrics.net_rx_mbps = (d_rx as f64 * 8.0 / 1_000_000.0) / delta_t;
                  metrics.net_tx_mbps = (d_tx as f64 * 8.0 / 1_000_000.0) / delta_t;

                  // Prunes
                  let d_prunes = s_sub(raw.net_prune_called, self.prev_snapshot.net_prune_called);
                  metrics.net_prunes_per_sec = d_prunes as f64 / delta_t;

                  // Context Switches
                  let d_ctxt = s_sub(raw.context_switches, self.prev_snapshot.context_switches);
                  metrics.context_switches_per_sec = d_ctxt as f64 / delta_t;
              }
        }
        
        metrics.net_rx_backlog = raw.net_rx_queue;
        metrics.rss_mem_mb = raw.memory_rss_kb as f64 / 1024.0;
        metrics.timestamp = now;
        
        // Commit State (Transactional)
        self.prev_snapshot = raw;
        
        Ok(metrics)
    }
}

// Helper: Wrapping Subtraction Safe Helper
fn s_sub(curr: u64, prev: u64) -> u64 {
    if curr >= prev {
        curr - prev
    } else {
        // Wrap around!
        (u64::MAX - prev) + curr
    }
}
