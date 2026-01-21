use std::time::Duration;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use crate::AppState; 

pub struct TuiAgent;

impl TuiAgent {

    pub fn draw_ui(f: &mut Frame<'_>, state: &AppState) {
        let title_text = if state.server_online {
            " VORTEX COMMAND CENTER [THE FOREMAN] "
        } else {
            " VORTEX COMMAND CENTER (⚠ OFFLINE ⚠) "
        };

        let last_hw = state.metrics_history.back();
        let last_log = state.last_log_tick.as_ref();
        
        let uptime_secs = state.start_time.map(|t| t.elapsed().as_secs()).unwrap_or(0);
        let uptime_str = format!("{:02}:{:02}", uptime_secs / 60, uptime_secs % 60);
        let mode_str = if state.is_release { "RELEASE" } else { "DEBUG" };

        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3), // Section A: Header
                Constraint::Min(20),   // Sections B & C (Middle)
                Constraint::Length(8), // Section D: Diagnostics
            ].as_ref())
            .split(f.size());

        // --- SECTION A: MISSION HEADER ---
        let header_style = if state.server_online { Style::default().fg(Color::Cyan) } else { Style::default().fg(Color::Red) };
        let header_block = Block::default().borders(Borders::ALL).title(title_text).border_style(header_style);
        
        let header_text = format!(
            " UPTIME: {} | MODE: {} | THROUGHPUT: {:.0} ops/s (PEAK: {:.0}) | TOTAL OPS: {}",
            uptime_str, mode_str, state.throughput_instant, state.peak_throughput, state.total_acks
        );
        let header = Paragraph::new(Line::from(vec![
            Span::styled(header_text, Style::default().add_modifier(Modifier::BOLD))
        ])).block(header_block);
        f.render_widget(header, main_chunks[0]);

        // Middle Row: B (Engine) and C (Hardware)
        let middle_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
            .split(main_chunks[1]);

        // --- SECTION B: ENGINE DYNAMICS ---
        let mut engine_lines = vec![];
        
        // Batch Saturation Bar
        let batch_bytes = last_log.map(|l| l.bytes).unwrap_or(0);
        let batch_sat = (batch_bytes as f64 / 262144.0 * 100.0).min(100.0);
        let sat_bar = format!("[{:_<20}] {:.1}%", "#".repeat((batch_sat / 5.0) as usize), batch_sat);
        engine_lines.push(Line::from(vec![Span::styled(" [ BATCH SATURATION ] ", Style::default().add_modifier(Modifier::BOLD)), Span::raw(sat_bar)]));
        
        // Flush Ratios
        let f_full = last_log.map(|l| l.flushes_full).unwrap_or(0);
        let f_eot = last_log.map(|l| l.flushes_eot).unwrap_or(0);
        engine_lines.push(Line::from(vec![Span::raw(format!("  FLUSHES: FULL={} | EOT={} (Ratio: {:.1})", f_full, f_eot, f_full as f64 / f_eot.max(1) as f64))]));
        
        // WAF
        let disk_bytes = last_hw.map(|s| s.disk_write_mb_s * 1048576.0).unwrap_or(0.0);
        let logical_bytes = last_log.map(|l| l.bytes as f64).unwrap_or(0.0);
        let waf = if logical_bytes > 0.0 { disk_bytes / logical_bytes } else { 0.0 };
        engine_lines.push(Line::from(vec![
            Span::raw("  WAF: "), 
            Span::styled(format!("{:.2}x", waf), if waf > 2.0 { Style::default().fg(Color::Yellow) } else { Style::default().fg(Color::Green) }),
            Span::raw(" (Disk/App Ratio)")
        ]));
        
        engine_lines.push(Line::from(vec![Span::raw("")]));
        
        // Search Stats
        let search = state.search_stats.as_ref();
        engine_lines.push(Line::from(vec![Span::styled(" [ SEARCH PERFORMANCE ] ", Style::default().add_modifier(Modifier::BOLD))]));
        if let Some(s) = search {
            let avg_lat = if s.ops > 0 { s.time_us as f64 / s.ops as f64 } else { 0.0 };
            engine_lines.push(Line::from(vec![Span::raw(format!("  QPS: {} ops/s | AVG LATENCY: {:.1} us", s.ops, avg_lat))]));
            engine_lines.push(Line::from(vec![Span::raw(format!("  DIST CALCS/SEC: {} (Work Metric)", s.dist_calcs))]));
        } else {
            engine_lines.push(Line::from(vec![Span::raw("  Waiting for search traffic...")]));
        }
        
        let engine_panel = Paragraph::new(engine_lines).block(Block::default().title(" II. ENGINE DYNAMICS ").borders(Borders::ALL));
        f.render_widget(engine_panel, middle_chunks[0]);

        // --- SECTION C: HARDWARE STRESS ---
        let mut hw_lines = vec![];
        let cpu_cores = last_hw.map(|s| &s.cpu_usage_pct).cloned().unwrap_or_default();
        let cpu_user = last_hw.map(|s| &s.cpu_user_pct).cloned().unwrap_or_default();
        let cpu_sys = last_hw.map(|s| &s.cpu_system_pct).cloned().unwrap_or_default();
        let cpu_soft = last_hw.map(|s| &s.cpu_softirq_pct).cloned().unwrap_or_default();

        hw_lines.push(Line::from(vec![Span::styled(" [ CORE UTILIZATION ] ", Style::default().add_modifier(Modifier::BOLD))]));
        for (i, util) in cpu_cores.iter().enumerate().take(4) {
            let bar = format!("[{:_<10}]", "#".repeat((util / 10.0) as usize));
            hw_lines.push(Line::from(vec![
                Span::raw(format!("  C{:02}: ", i)),
                Span::styled(bar, Style::default().fg(if *util > 90.0 { Color::Red } else { Color::Cyan })),
                Span::raw(format!(" {:>5.1}% (U:{:.0}% S:{:.0}% SI:{:.0}%)", 
                    util, cpu_user.get(i).unwrap_or(&0.0), cpu_sys.get(i).unwrap_or(&0.0), cpu_soft.get(i).unwrap_or(&0.0)))
            ]));
        }
        
        hw_lines.push(Line::from(vec![Span::raw("")]));
        let ctxt = last_hw.map(|s| s.context_switches_per_sec).unwrap_or(0.0);
        hw_lines.push(Line::from(vec![Span::raw(format!("  CONTXT SWITCHES/S: {:.0}", ctxt))]));
        hw_lines.push(Line::from(vec![Span::raw(format!("  RSS MEMORY: {:.1} MB (PEAK: {:.1} MB)", 
            last_hw.map(|s| s.rss_mem_mb).unwrap_or(0.0), state.peak_rss_mb))]));

        // Shard Health / Contention
        hw_lines.push(Line::from(vec![Span::raw("")]));
        hw_lines.push(Line::from(vec![Span::styled(" [ SHARD HEALTH ] ", Style::default().add_modifier(Modifier::BOLD))]));
        if let Some(h) = state.health_stats.as_ref() {
             let total_tick = (h.ingress_ms + h.flush_ms).max(1);
             let ingress_ratio = h.ingress_ms as f64 / total_tick as f64 * 100.0;
             let flush_ratio = h.flush_ms as f64 / total_tick as f64 * 100.0;
             hw_lines.push(Line::from(vec![Span::raw(format!("  CYCLE STARVATION: Log={:.0}% | Persistence={:.0}%", ingress_ratio, flush_ratio))]));
             if h.flush_ms > 20 {
                 hw_lines.push(Line::from(vec![Span::styled("  ⚠ READ LATENCY RISK: Flush Stall > 20ms", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))]));
             }
        }

        let hw_panel = Paragraph::new(hw_lines).block(Block::default().title(" III. HARDWARE STRESS ").borders(Borders::ALL));
        f.render_widget(hw_panel, middle_chunks[1]);

        // --- SECTION D: DIAGNOSTICS ---
        let diag_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
            .split(main_chunks[2]);

        let mut net_lines = vec![];
        let rx_mbps = last_hw.map(|s| s.net_rx_mbps).unwrap_or(0.0);
        let tx_mbps = last_hw.map(|s| s.net_tx_mbps).unwrap_or(0.0);
        let backlog = last_hw.map(|s| s.net_rx_backlog).unwrap_or(0);
        net_lines.push(Line::from(vec![Span::styled(" [ NETWORK ] ", Style::default().add_modifier(Modifier::BOLD)), 
            Span::raw(format!("RX: {:.1} Mbps | TX: {:.1} Mbps | Backlog: {} bytes", rx_mbps, tx_mbps, backlog))]));
        
        let packet_overhead = if rx_mbps > 0.0 { (logical_bytes * 8.0 / 1_000_000.0) / rx_mbps } else { 0.0 };
        net_lines.push(Line::from(vec![Span::raw(format!("  EFFICIENCY: {:.1}% (VBP Payload / Raw Wire)", packet_overhead * 100.0))]));
        
        let net_panel = Paragraph::new(net_lines).block(Block::default().title(" IV. NETWORK DIAGNOSTICS ").borders(Borders::ALL));
        f.render_widget(net_panel, diag_chunks[0]);

        // Disk/Verdict (Re-branded as LIVE RECEIPT)
        let disk_mb_s = last_hw.map(|s| s.disk_write_mb_s).unwrap_or(0.0);
        let mut io_lines = vec![
            Line::from(vec![Span::styled(" [ STORAGE ] ", Style::default().add_modifier(Modifier::BOLD)), Span::raw(format!("{:.2} MB/s", disk_mb_s))]),
        ];

        if let Some(worker) = &state.worker_stats {
            let stale = state.last_worker_update.map(|t| t.elapsed() > Duration::from_secs(3)).unwrap_or(true);
            let color = if stale { Color::DarkGray } else { Color::Cyan };
            let status_text = if stale { format!("IDLE ({})", worker.name) } else { worker.name.clone() };
            
            io_lines.push(Line::from(vec![
                Span::styled(format!(" [ WORKER: {} ]", status_text), Style::default().add_modifier(Modifier::BOLD).fg(color))
            ]));
            
            let drop_color = if worker.drops > 0 { Color::Red } else { Color::Green };
            io_lines.push(Line::from(vec![
                Span::raw(" ACKs: "), Span::styled(format!("{}/{}", worker.acks, worker.target), Style::default().fg(Color::Yellow)),
                Span::raw(" | Drops: "), Span::styled(worker.drops.to_string(), Style::default().fg(drop_color)),
            ]));
            
            io_lines.push(Line::from(vec![
                Span::raw(" P50: "), Span::styled(format!("{}us", worker.p50_us), Style::default().fg(Color::Cyan)),
                Span::raw(" | P99: "), Span::styled(format!("{}us", worker.p99_us), Style::default().fg(Color::Magenta)),
            ]));
        } else {
             io_lines.push(Line::from(vec![Span::styled(" [ WORKER: WAITING... ]", Style::default().fg(Color::DarkGray))]));
             io_lines.push(Line::from(vec![Span::raw("  Launch stress_test to see live P99 stats.")]));
        }

        let io_panel = Paragraph::new(io_lines).block(Block::default().title(" V. LIVE RECEIPT ").borders(Borders::ALL));
        f.render_widget(io_panel, diag_chunks[1]);
    }
}

// Wrapper for main.rs to call
pub fn draw_ui(f: &mut Frame<'_>, state: &AppState) {
    TuiAgent::draw_ui(f, state);
}
