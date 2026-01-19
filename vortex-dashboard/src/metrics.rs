use std::collections::VecDeque;
use std::fs;
use std::time::Instant;
use anyhow::{Result, anyhow};

pub struct MetricsSnapshot {
    #[allow(dead_code)]
    pub timestamp: Instant,
    pub cpu_user: f64,
    pub cpu_sys: f64,
    pub cpu_cores: Vec<f64>,
    pub throughput_ops: f64,
    #[allow(dead_code)]
    pub disk_write_kb: f64,
    pub socket_q_depth: usize,
    #[allow(dead_code)]
    pub context_switches: u64,
    pub batch_size_avg: f64,
    pub rss_kb: u64,
}

pub struct MetricsState {
    pub history: VecDeque<MetricsSnapshot>,
    pub total_acks: usize,
    pub start_time: Instant,
    
    // Phase 9: Mission Control
    pub peak_throughput: f64,
    pub eot_flushes: usize,
    pub full_flushes: usize,
    pub backpressure_events: usize,
    pub shard_pulses: Vec<Instant>,
    
    // Internal counters for deltas
    last_disk_sectors: u64,
    last_context_switches: u64,
    last_sample_time: Instant,
    
    // Phase 9: CPU delta tracking (total, work, system)
    last_cpu_total: Vec<u64>,
    last_cpu_work: Vec<u64>,
    last_cpu_sys: Vec<u64>,
}

impl MetricsState {
    pub fn new() -> Self {
        Self {
            history: VecDeque::with_capacity(1000),
            total_acks: 0,
            start_time: Instant::now(),
            peak_throughput: 0.0,
            eot_flushes: 0,
            full_flushes: 0,
            backpressure_events: 0,
            shard_pulses: vec![Instant::now(); 2], // Default to 2 shards
            last_disk_sectors: 0,
            last_context_switches: 0,
            last_sample_time: Instant::now(),
            last_cpu_total: vec![0; 5], // Total + Core 0-3
            last_cpu_work: vec![0; 5],
            last_cpu_sys: vec![0; 5],
        }
    }

    pub fn check_perf_permissions() -> Result<()> {
        let paranoid = fs::read_to_string("/proc/sys/kernel/perf_event_paranoid")
            .unwrap_or_else(|_| "2".to_string())
            .trim()
            .parse::<i32>()
            .unwrap_or(2);
        
        if paranoid >= 2 {
            return Err(anyhow!("Kernel blocked perf_event_open (perf_event_paranoid={}). Run: 'sudo sysctl -w kernel.perf_event_paranoid=1'", paranoid));
        }
        Ok(())
    }

    pub fn sample(&mut self, server_pid: u32, batch_throughput: f64, batch_size: f64) -> Result<()> {
        let now = Instant::now();
        let delta_t = now.duration_since(self.last_sample_time).as_secs_f64();
        if delta_t == 0.0 { return Ok(()); }

        // 3. Socket Queues (/proc/net/tcp) - Targeted for Port 9000 (0x2328)
        let tcp = fs::read_to_string("/proc/net/tcp")?;
        let mut q_depth = 0;
        let target_port_hex = ":2328"; // 9000 in hex
        for line in tcp.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() > 4 && parts[1].ends_with(target_port_hex) {
                // Column 4 is rx_queue:tx_queue (hex)
                if let Some(queue_part) = parts.get(3) { // It's actually column 4 (index 3) in most proc net tcp
                     let queues: Vec<&str> = queue_part.split(':').collect();
                     if queues.len() == 2 {
                         q_depth = usize::from_str_radix(queues[0], 16).unwrap_or(0);
                         break; // We found our target listener
                     }
                }
            }
        }

        // 4. Per-Core CPU & Syscall Efficiency (/proc/stat)
        let stat = fs::read_to_string("/proc/stat")?;
        let mut core_utils = Vec::new();
        let mut sys_utils = Vec::new();
        let mut cpu_idx = 0;
        for line in stat.lines() {
            if line.starts_with("cpu") && cpu_idx < 5 {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 5 {
                    let user: u64 = parts[1].parse().unwrap_or(0);
                    let nice: u64 = parts[2].parse().unwrap_or(0);
                    let system: u64 = parts[3].parse().unwrap_or(0);
                    let idle: u64 = parts[4].parse().unwrap_or(0);
                    let iowait: u64 = parts[5].parse().unwrap_or(0);
                    
                    let work = user + nice + system;
                    let total = work + idle + iowait;
                    
                    if self.last_cpu_total[cpu_idx] > 0 {
                        let diff_total = total - self.last_cpu_total[cpu_idx];
                        let diff_work = work - self.last_cpu_work[cpu_idx];
                        let diff_sys = system - self.last_cpu_sys[cpu_idx];
                        
                        let util = (diff_work as f64 / diff_total as f64) * 100.0;
                        let sys_util = (diff_sys as f64 / diff_total as f64) * 100.0;
                        core_utils.push(util);
                        sys_utils.push(sys_util);
                    } else {
                        core_utils.push(0.0);
                        sys_utils.push(0.0);
                    }
                    self.last_cpu_total[cpu_idx] = total;
                    self.last_cpu_work[cpu_idx] = work;
                    self.last_cpu_sys[cpu_idx] = system;
                    cpu_idx += 1;
                }
            }
        }
        
        let global_cpu = core_utils.get(0).cloned().unwrap_or(0.0);
        let cpu_sys = sys_utils.get(0).cloned().unwrap_or(0.0);
        let core_utils_final = core_utils.into_iter().skip(1).collect(); // Keep Core 0-3

        // 5. RSS Memory & Context Switches (/proc/[pid]/status)
        let status = fs::read_to_string(format!("/proc/{}/status", server_pid))?;
        let mut rss_kb = 0;
        let mut voluntary = 0u64;
        let mut non_voluntary = 0u64;
        for line in status.lines() {
            if line.starts_with("VmRSS:") {
                rss_kb = line.split_whitespace().nth(1).unwrap_or("0").parse().unwrap_or(0);
            }
            if line.starts_with("voluntary_ctxt_switches:") {
                voluntary = line.split_whitespace().last().unwrap_or("0").parse()?;
            }
            if line.starts_with("nonvoluntary_ctxt_switches:") {
                non_voluntary = line.split_whitespace().last().unwrap_or("0").parse()?;
            }
        }
        let total_cs = voluntary + non_voluntary;
        let cs_rate = if self.last_context_switches > 0 {
            ((total_cs - self.last_context_switches) as f64 / delta_t) as u64
        } else { 0 };
        self.last_context_switches = total_cs;

        // 2. Disk Stats (/proc/diskstats)
        let diskstats = fs::read_to_string("/proc/diskstats")?;
        let mut total_sectors_written = 0u64;
        for line in diskstats.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() > 9 {
                total_sectors_written += parts[parts.len() - 5].parse::<u64>().unwrap_or(0); // 10th column (index 9) is sectors written
            }
        }
        let disk_kb = if self.last_disk_sectors > 0 {
            let delta_sectors = total_sectors_written.saturating_sub(self.last_disk_sectors);
            (delta_sectors as f64 * 0.5) / delta_t
        } else { 0.0 };
        self.last_disk_sectors = total_sectors_written;

        // Push to history
        self.history.push_back(MetricsSnapshot {
            timestamp: now,
            cpu_user: global_cpu,
            cpu_sys,
            cpu_cores: core_utils_final,
            throughput_ops: batch_throughput,
            disk_write_kb: disk_kb,
            socket_q_depth: q_depth,
            context_switches: cs_rate,
            batch_size_avg: batch_size,
            rss_kb,
        });

        if self.history.len() > 600 { // 60 seconds at 10Hz
            self.history.pop_front();
        }

        self.last_sample_time = now;
        Ok(())
    }
}
