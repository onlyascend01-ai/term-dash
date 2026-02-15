use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Row, Sparkline, Table, Tabs},
    Terminal,
};
use std::{collections::VecDeque, io, time::{Duration, Instant}};
use sysinfo::{Networks, System, Disks};

const TICK_RATE: u64 = 1000;
const HISTORY_LEN: usize = 100; // Keep last 100 ticks for graphs

struct App {
    system: System,
    networks: Networks,
    disks: Disks,
    cpu_history: VecDeque<u64>,
    mem_history: VecDeque<u64>,
    should_quit: bool,
}

impl App {
    fn new() -> Self {
        let mut system = System::new_all();
        let networks = Networks::new_with_refreshed_list();
        let disks = Disks::new_with_refreshed_list();
        system.refresh_all();
        
        Self {
            system,
            networks,
            disks,
            cpu_history: VecDeque::from(vec![0; HISTORY_LEN]),
            mem_history: VecDeque::from(vec![0; HISTORY_LEN]),
            should_quit: false,
        }
    }

    fn on_tick(&mut self) {
        self.system.refresh_all();
        self.networks.refresh(); 
        self.disks.refresh_list();

        // Update CPU History
        let cpu_usage = self.system.global_cpu_info().cpu_usage() as u64;
        self.cpu_history.pop_front();
        self.cpu_history.push_back(cpu_usage);

        // Update Memory History
        let total_mem = self.system.total_memory();
        let used_mem = self.system.used_memory();
        let mem_percent = if total_mem > 0 {
            (used_mem as f64 / total_mem as f64 * 100.0) as u64
        } else {
            0
        };
        self.mem_history.pop_front();
        self.mem_history.push_back(mem_percent);
    }
}

fn main() -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let mut app = App::new();
    let tick_rate = Duration::from_millis(TICK_RATE);
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|f| ui(f, &mut app))?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if crossterm::event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
                        _ => {}
                    }
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            app.on_tick();
            last_tick = Instant::now();
        }

        if app.should_quit {
            break;
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;

    Ok(())
}

fn ui(f: &mut ratatui::Frame, app: &mut App) {
    // Main layout
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),      // Header
            Constraint::Percentage(30), // Graphs (Sparklines)
            Constraint::Percentage(20), // Gauges (Immediate value)
            Constraint::Percentage(50), // Tables (Disk/Net)
        ])
        .split(f.area());

    // 1. Header
    let host_name = System::host_name().unwrap_or_else(|| "Unknown".to_string());
    let os_name = System::name().unwrap_or_else(|| "Unknown".to_string());
    let uptime_seconds = System::uptime();
    let hours = uptime_seconds / 3600;
    let minutes = (uptime_seconds % 3600) / 60;
    
    let header_text = Line::from(vec![
        Span::styled(" TERM-DASH ", Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(format!(" | {} | {} | Uptime: {}h {}m ", host_name, os_name, hours, minutes)),
        Span::styled(" [Q] Quit ", Style::default().fg(Color::DarkGray)),
    ]);

    let header = Paragraph::new(header_text)
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Cyan)))
        .alignment(ratatui::layout::Alignment::Center);
    
    f.render_widget(header, chunks[0]);

    // 2. Graphs Section (Sparklines)
    let graph_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[1]);

    // CPU Sparkline
    let cpu_data: Vec<u64> = app.cpu_history.iter().cloned().collect();
    let cpu_sparkline = Sparkline::default()
        .block(Block::default().title(" CPU History ").borders(Borders::LEFT | Borders::RIGHT | Borders::TOP))
        .data(&cpu_data)
        .style(Style::default().fg(Color::Green));
    f.render_widget(cpu_sparkline, graph_chunks[0]);

    // RAM Sparkline
    let mem_data: Vec<u64> = app.mem_history.iter().cloned().collect();
    let mem_sparkline = Sparkline::default()
        .block(Block::default().title(" Memory History ").borders(Borders::LEFT | Borders::RIGHT | Borders::TOP))
        .data(&mem_data)
        .style(Style::default().fg(Color::Magenta));
    f.render_widget(mem_sparkline, graph_chunks[1]);

    // 3. Gauges Section (Immediate Values)
    let gauge_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[2]);

    // CPU Gauge
    let cpu_val = *app.cpu_history.back().unwrap_or(&0);
    let cpu_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Green)))
        .gauge_style(Style::default().fg(if cpu_val > 80 { Color::Red } else { Color::Green }))
        .percent(cpu_val as u16)
        .label(format!("CPU: {}%", cpu_val));
    f.render_widget(cpu_gauge, gauge_chunks[0]);

    // RAM Gauge
    let mem_val = *app.mem_history.back().unwrap_or(&0);
    let total_mem_gb = app.system.total_memory() as f64 / 1_073_741_824.0;
    let used_mem_gb = app.system.used_memory() as f64 / 1_073_741_824.0;
    
    let mem_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Magenta)))
        .gauge_style(Style::default().fg(Color::Magenta))
        .percent(mem_val as u16)
        .label(format!("MEM: {:.1} / {:.1} GB ({}%)", used_mem_gb, total_mem_gb, mem_val));
    f.render_widget(mem_gauge, gauge_chunks[1]);

    // 4. Bottom Section: Disks & Network
    let bottom_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(chunks[3]);

    // Disk Table
    let disks = &app.disks;
    let mut disk_rows = Vec::new();
    for disk in disks {
        let total = disk.total_space();
        let available = disk.available_space();
        let used = total - available;
        let percent = if total > 0 { (used as f64 / total as f64 * 100.0) as u16 } else { 0 };

        let color = if percent > 90 { Color::Red } else if percent > 75 { Color::Yellow } else { Color::Green };
        
        disk_rows.push(Row::new(vec![
            format!("{:?}", disk.mount_point()),
            format!("{:.1} GB", total as f64 / 1_073_741_824.0),
            format!("{}%", percent),
        ]).style(Style::default().fg(color)));
    }

    let disk_table = Table::new(disk_rows, [
        Constraint::Percentage(40),
        Constraint::Percentage(30),
        Constraint::Percentage(30),
    ])
    .header(Row::new(vec!["Mount", "Total", "Used"]).style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)))
    .block(Block::default().borders(Borders::ALL).title(" Storage "));

    f.render_widget(disk_table, bottom_chunks[0]);

    // Network Table
    let mut net_rows = Vec::new();
    for (interface_name, data) in &app.networks {
        // Only show active interfaces to reduce clutter
        if data.received() > 0 || data.transmitted() > 0 {
             net_rows.push(Row::new(vec![
                interface_name.clone(),
                format!("{:.1} KB", data.received() as f64 / 1024.0),
                format!("{:.1} KB", data.transmitted() as f64 / 1024.0),
            ]));
        }
    }

    let net_table = Table::new(net_rows, [
        Constraint::Percentage(40),
        Constraint::Percentage(30),
        Constraint::Percentage(30),
    ])
    .header(Row::new(vec!["Interface", "RX (KB)", "TX (KB)"]).style(Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD)))
    .block(Block::default().borders(Borders::ALL).title(" Network Activity "));

    f.render_widget(net_table, bottom_chunks[1]);
}
