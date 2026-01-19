use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Table, Row, Cell},
    Frame,
};
use crate::metrics::MetricsState;

pub struct TuiAgent {
    pub title: String,
}

impl TuiAgent {
    pub fn new() -> Self {
        Self {
            title: "VORTEX COMMAND CENTER (ELITE MISSION CONTROL)".to_string(),
        }
    }

    pub fn draw(&self, f: &mut Frame<'_>, state: &MetricsState) {
        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints(
                [
                    Constraint::Length(3), // Header
                    Constraint::Min(10),   // Main Dynamics (3-panel)
                    Constraint::Length(6), // Heartbeat Pulse
                    Constraint::Length(3), // Verdict
                ]
                .as_ref(),
            )
            .split(f.size());

        let last = state.history.back();
        let time_elapsed = state.start_time.elapsed().as_secs_f64();
        let global_avg = state.total_acks as f64 / time_elapsed.max(1.0);

        // 1. Header
        let header = Block::default()
            .borders(Borders::ALL)
            .title(self.title.as_str())
            .border_style(Style::default().fg(Color::Cyan));
        let header_content = Paragraph::new(format!(" Uptime: {:.1}s | Total ACKs: {} | Global Avg: {:.2} ops/sec | Peak: {:.2} ops/sec", 
            time_elapsed, state.total_acks, global_avg, state.peak_throughput))
            .block(header);
        f.render_widget(header_content, main_chunks[0]);

        // 2. Main Mission Control (3-way split)
        let ctrl_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(33), Constraint::Percentage(34), Constraint::Percentage(33)].as_ref())
            .split(main_chunks[1]);

        // --- Panel 1: Engine Dynamics ---
        let saturation = last.map(|s| (s.batch_size_avg * 1024.0 / 262144.0 * 100.0).min(100.0)).unwrap_or(0.0);
        let backpressure_rate = state.backpressure_events as f64 / time_elapsed.max(1.0);
        
        let engine_content = vec![
            Line::from(vec![Span::raw(" [ BATCH SATURATION ]")]),
            Line::from(vec![Span::styled(format!("  {:.1}%", saturation), Style::default().fg(Color::Green))]),
            Line::from(vec![Span::raw("")]),
            Line::from(vec![Span::raw(" [ FLUSH REASONS ]")]),
            Line::from(vec![Span::styled(format!("  FULL: {}  |  EOT: {}", state.full_flushes, state.eot_flushes), Style::default().fg(Color::Yellow))]),
            Line::from(vec![Span::raw("")]),
            Line::from(vec![Span::raw(" [ BACKPRESSURE ]")]),
            Line::from(vec![Span::styled(format!("  {:.2} events/sec", backpressure_rate), 
                if backpressure_rate > 0.0 { Style::default().fg(Color::Red) } else { Style::default().fg(Color::Green) })]),
        ];

        let engine_panel = Paragraph::new(engine_content)
            .block(Block::default().title(" I. ENGINE DYNAMICS ").borders(Borders::ALL));
        f.render_widget(engine_panel, ctrl_chunks[0]);

        // --- Panel 2: Hardware Stress ---
        let cpu_cores = last.map(|s| &s.cpu_cores).cloned().unwrap_or_default();
        let rss_mb = last.map(|s| s.rss_kb as f64 / 1024.0).unwrap_or(0.0);
        
        let mut hardware_lines = vec![
            Line::from(vec![Span::raw(" [ PER-CORE UTILIZATION ]")]),
        ];
        
        for (i, util) in cpu_cores.iter().enumerate().take(4) {
            hardware_lines.push(Line::from(vec![
                Span::raw(format!("  CORE {}: ", i)),
                Span::styled(format!("{:.1}% ", util), if *util > 90.0 { Style::default().fg(Color::Red) } else { Style::default().fg(Color::Cyan) }),
            ]));
        }
        
        hardware_lines.push(Line::from(vec![Span::raw("")]));
        let cpu_sys_val = last.map(|s| s.cpu_sys).unwrap_or(0.0);
        let cpu_user_val = last.map(|s| s.cpu_user).unwrap_or(0.0);
        let total_cpu = cpu_sys_val + cpu_user_val;
        let sys_ratio = if total_cpu > 0.0 { (cpu_sys_val / total_cpu) * 100.0 } else { 0.0 };
        
        hardware_lines.push(Line::from(vec![Span::raw(" [ SYSCALL EFFICIENCY ]")]));
        hardware_lines.push(Line::from(vec![
            Span::raw(format!("  User: {:.1}% | Sys: ", cpu_user_val)),
            Span::styled(format!("{:.1}%", cpu_sys_val), if sys_ratio > 15.0 { Style::default().fg(Color::Red) } else { Style::default().fg(Color::Green) }),
        ]));

        hardware_lines.push(Line::from(vec![Span::raw("")]));
        hardware_lines.push(Line::from(vec![Span::raw(" [ RSS MEMORY ]")]));
        hardware_lines.push(Line::from(vec![Span::styled(format!("  {:.1} MB", rss_mb), Style::default().fg(Color::Magenta))]));

        let hardware_panel = Paragraph::new(hardware_lines)
            .block(Block::default().title(" II. HARDWARE STRESS ").borders(Borders::ALL));
        f.render_widget(hardware_panel, ctrl_chunks[1]);

        // --- Panel 3: Network Diagnostics ---
        let q_depth = last.map(|s| s.socket_q_depth).unwrap_or(0);
        let inst_throughput = last.map(|s| s.throughput_ops).unwrap_or(0.0);
        let latency_est = if inst_throughput > 0.0 { (32.0 / inst_throughput) * 1000.0 } else { 0.0 };

        let network_content = vec![
            Line::from(vec![Span::raw(" [ RECV-QUEUE ]")]),
            Line::from(vec![Span::styled(format!("  {} packets", q_depth), if q_depth > 100 { Style::default().fg(Color::Red) } else { Style::default().fg(Color::Green) })]),
            Line::from(vec![Span::raw("")]),
            Line::from(vec![Span::raw(" [ LITTLE'S LAW LATENCY ]")]),
            Line::from(vec![Span::styled(format!("  {:.2} ms (theoretical)", latency_est), Style::default().fg(Color::Yellow))]),
            Line::from(vec![Span::raw("")]),
            Line::from(vec![Span::raw(" [ INSTANT FLOW ]")]),
            Line::from(vec![Span::styled(format!("  {:.2} ops/sec", inst_throughput), Style::default().fg(Color::Green))]),
        ];

        let network_panel = Paragraph::new(network_content)
            .block(Block::default().title(" III. NETWORK DIAGNOSTICS ").borders(Borders::ALL));
        f.render_widget(network_panel, ctrl_chunks[2]);

        // 3. Heartbeat Pulse
        let rows: Vec<Row> = state.shard_pulses.iter().enumerate().map(|(id, pulse)| {
            let elapsed = pulse.elapsed().as_secs_f64();
            let status = if elapsed > 2.0 { "STALLED" } else { "ACTIVE" };
            let style = if status == "STALLED" { Style::default().fg(Color::Red).add_modifier(Modifier::SLOW_BLINK) } else { Style::default().fg(Color::Green) };
            
            Row::new(vec![
                Cell::from(format!(" SHARD {}", id)),
                Cell::from(status).style(style),
                Cell::from(format!("{:.1}s ago", elapsed)),
                Cell::from(if status == "ACTIVE" { "♥♥♥" } else { "---" }).style(style),
            ])
        }).collect();

        let pulse_table = Table::new(rows, [Constraint::Percentage(25), Constraint::Percentage(25), Constraint::Percentage(25), Constraint::Percentage(25)].as_ref())
            .header(Row::new(vec!["UNIT", "STATUS", "LAST PULSE", "HEARTBEAT"]).style(Style::default().add_modifier(Modifier::BOLD)))
            .block(Block::default().title(" SHARD PULSE MONITOR ").borders(Borders::ALL));
        f.render_widget(pulse_table, main_chunks[2]);

        // 4. Final Verdict
        let verdict_text = if state.total_acks >= 80000 { 
            format!("SUCCESS: SATURATION BURN OVER. FINAL THROUGHPUT: {:.2} ops/sec", global_avg)
        } else { 
            "STATUS: VORTEX ENGINE ONLINE - MONITORING BURN...".to_string() 
        };
        
        let verdict_color = if state.total_acks >= 80000 { Color::Cyan } else { Color::White };
        let verdict = Paragraph::new(verdict_text)
            .block(Block::default().borders(Borders::ALL))
            .style(Style::default().fg(verdict_color).add_modifier(Modifier::BOLD));
        f.render_widget(verdict, main_chunks[3]);
    }
}
