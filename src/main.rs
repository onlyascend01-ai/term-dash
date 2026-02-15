use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, Paragraph, Row, Sparkline, Table, TableState, Wrap},
    Terminal,
};
use std::{collections::VecDeque, io, time::{Duration, Instant}};
use sysinfo::{
    CpuRefreshKind, Disks, MemoryRefreshKind, Networks, ProcessRefreshKind, RefreshKind, System, Pid,
};

const TICK_RATE: u64 = 1000;
const HISTORY_LEN: usize = 100;

#[derive(Clone, Copy, PartialEq)]
enum InputMode {
    Normal,
    Editing,
    Details, // New mode for Process Inspector
}

#[derive(Clone, Copy)]
enum ThemePreset {
    Default,
    Cyberpunk,
    Matrix,
}

impl ThemePreset {
    fn next(&self) -> Self {
        match self {
            ThemePreset::Default => ThemePreset::Cyberpunk,
            ThemePreset::Cyberpunk => ThemePreset::Matrix,
            ThemePreset::Matrix => ThemePreset::Default,
        }
    }

    fn get_theme(&self) -> Theme {
        match self {
            ThemePreset::Default => Theme {
                bg: Color::Reset,
                border: Color::Cyan,
                text: Color::White,
                highlight_fg: Color::White,
                highlight_bg: Color::Red,
                graph_cpu: Color::Green,
                graph_mem: Color::Magenta,
                graph_net_rx: Color::Yellow,
                graph_net_tx: Color::Blue,
                gauge_cpu_high: Color::Red,
                gauge_cpu_low: Color::Green,
                gauge_mem: Color::Magenta,
            },
            ThemePreset::Cyberpunk => Theme {
                bg: Color::Black,
                border: Color::Magenta,
                text: Color::Cyan,
                highlight_fg: Color::Black,
                highlight_bg: Color::Yellow,
                graph_cpu: Color::LightMagenta,
                graph_mem: Color::LightCyan,
                graph_net_rx: Color::LightGreen,
                graph_net_tx: Color::LightYellow,
                gauge_cpu_high: Color::Red,
                gauge_cpu_low: Color::LightMagenta,
                gauge_mem: Color::LightCyan,
            },
            ThemePreset::Matrix => Theme {
                bg: Color::Black,
                border: Color::Green,
                text: Color::DarkGray,
                highlight_fg: Color::Black,
                highlight_bg: Color::Green,
                graph_cpu: Color::LightGreen,
                graph_mem: Color::Green,
                graph_net_rx: Color::LightGreen,
                graph_net_tx: Color::Green,
                gauge_cpu_high: Color::LightGreen,
                gauge_cpu_low: Color::DarkGray,
                gauge_mem: Color::Green,
            },
        }
    }
}

struct Theme {
    bg: Color,
    border: Color,
    text: Color,
    highlight_fg: Color,
    highlight_bg: Color,
    graph_cpu: Color,
    graph_mem: Color,
    graph_net_rx: Color,
    graph_net_tx: Color,
    gauge_cpu_high: Color,
    gauge_cpu_low: Color,
    gauge_mem: Color,
}

struct App {
    system: System,
    networks: Networks,
    disks: Disks,
    cpu_history: VecDeque<u64>,
    mem_history: VecDeque<u64>,
    net_rx_history: VecDeque<u64>,
    net_tx_history: VecDeque<u64>,
    should_quit: bool,
    // Process Interaction
    process_state: TableState,
    processes: Vec<(Pid, String, f32, u64)>, // Cache for list
    input_mode: InputMode,
    search_query: String,
    selected_pid: Option<Pid>, // Track which process is inspected
    current_theme: ThemePreset,
}

impl App {
    fn new() -> Self {
        let r = RefreshKind::new()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything())
            .with_processes(ProcessRefreshKind::everything());

        let mut system = System::new_with_specifics(r);
        let networks = Networks::new_with_refreshed_list();
        let disks = Disks::new_with_refreshed_list();
        system.refresh_all();
        
        let mut process_state = TableState::default();
        process_state.select(Some(0));

        Self {
            system,
            networks,
            disks,
            cpu_history: VecDeque::from(vec![0; HISTORY_LEN]),
            mem_history: VecDeque::from(vec![0; HISTORY_LEN]),
            net_rx_history: VecDeque::from(vec![0; HISTORY_LEN]),
            net_tx_history: VecDeque::from(vec![0; HISTORY_LEN]),
            should_quit: false,
            process_state,
            processes: Vec::new(),
            input_mode: InputMode::Normal,
            search_query: String::new(),
            selected_pid: None,
            current_theme: ThemePreset::Default,
        }
    }

    fn on_tick(&mut self) {
        self.system.refresh_all();
        self.networks.refresh(); 
        self.disks.refresh_list();

        // Update History
        let cpu_usage = self.system.global_cpu_info().cpu_usage() as u64;
        self.cpu_history.pop_front();
        self.cpu_history.push_back(cpu_usage);

        let total_mem = self.system.total_memory();
        let used_mem = self.system.used_memory();
        let mem_percent = if total_mem > 0 {
            (used_mem as f64 / total_mem as f64 * 100.0) as u64
        } else {
            0
        };
        self.mem_history.pop_front();
        self.mem_history.push_back(mem_percent);

        // Update Network History
        let mut total_rx = 0;
        let mut total_tx = 0;
        for (_, data) in &self.networks {
            total_rx += data.received();
            total_tx += data.transmitted();
        }
        self.net_rx_history.pop_front();
        self.net_rx_history.push_back(total_rx);
        self.net_tx_history.pop_front();
        self.net_tx_history.push_back(total_tx);

        // Update Process Cache
        let mut procs: Vec<_> = self.system.processes().values().collect();
        
        if !self.search_query.is_empty() {
            procs.retain(|p| p.name().to_lowercase().contains(&self.search_query.to_lowercase()));
            procs.sort_by(|a, b| b.cpu_usage().partial_cmp(&a.cpu_usage()).unwrap_or(std::cmp::Ordering::Equal));
        } else {
            procs.sort_by(|a, b| b.cpu_usage().partial_cmp(&a.cpu_usage()).unwrap_or(std::cmp::Ordering::Equal));
            procs.truncate(50); // Increased list size
        }
        
        self.processes = procs.iter().map(|p| (
            p.pid(), 
            p.name().to_string(), 
            p.cpu_usage(), 
            p.memory()
        )).collect();
    }

    fn next_process(&mut self) {
        if self.processes.is_empty() { return; }
        let i = match self.process_state.selected() {
            Some(i) => if i >= self.processes.len() - 1 { 0 } else { i + 1 },
            None => 0,
        };
        self.process_state.select(Some(i));
    }

    fn previous_process(&mut self) {
        if self.processes.is_empty() { return; }
        let i = match self.process_state.selected() {
            Some(i) => if i == 0 { self.processes.len() - 1 } else { i - 1 },
            None => 0,
        };
        self.process_state.select(Some(i));
    }

    fn kill_selected_process(&mut self) {
        if let Some(i) = self.process_state.selected() {
            if let Some((pid, _, _, _)) = self.processes.get(i) {
                if let Some(process) = self.system.process(*pid) {
                    process.kill();
                }
            }
        }
    }

    fn inspect_selected_process(&mut self) {
        if let Some(i) = self.process_state.selected() {
            if let Some((pid, _, _, _)) = self.processes.get(i) {
                self.selected_pid = Some(*pid);
                self.input_mode = InputMode::Details;
            }
        }
    }
}

fn main() -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

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
                    match app.input_mode {
                        InputMode::Normal => match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
                            KeyCode::Down | KeyCode::Char('j') => app.next_process(),
                            KeyCode::Up | KeyCode::Char('k') => app.previous_process(),
                            KeyCode::Char('x') | KeyCode::Delete => app.kill_selected_process(),
                            KeyCode::Char('/') => {
                                app.input_mode = InputMode::Editing;
                                app.process_state.select(Some(0)); 
                            }
                            KeyCode::Enter => app.inspect_selected_process(),
                            KeyCode::Char('t') => {
                                app.current_theme = app.current_theme.next();
                            }
                            _ => {}
                        },
                        InputMode::Editing => match key.code {
                            KeyCode::Enter | KeyCode::Esc => {
                                app.input_mode = InputMode::Normal;
                            }
                            KeyCode::Backspace => {
                                app.search_query.pop();
                            }
                            KeyCode::Char(c) => {
                                app.search_query.push(c);
                            }
                            _ => {}
                        },
                        InputMode::Details => match key.code {
                            KeyCode::Esc | KeyCode::Enter | KeyCode::Backspace => {
                                app.input_mode = InputMode::Normal;
                                app.selected_pid = None;
                            }
                            _ => {}
                        }
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

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;

    Ok(())
}

// Helper for centering the modal
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn ui(f: &mut ratatui::Frame, app: &mut App) {
    let theme = app.current_theme.get_theme();
    let area = f.area();
    
    // Set background color for the whole terminal
    let bg_block = Block::default().style(Style::default().bg(theme.bg));
    f.render_widget(bg_block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),      // Header
            Constraint::Percentage(40), // Top: Graphs + Processes
            Constraint::Percentage(20), // Gauges
            Constraint::Percentage(40), // Bottom: Disk + Net
        ])
        .split(area);

    // 1. Header
    let host_name = System::host_name().unwrap_or_else(|| "Unknown".to_string());
    let header_text = Line::from(vec![
        Span::styled(" TERM-DASH v0.5 ", Style::default().fg(theme.bg).bg(theme.border).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" | Host: {} ", host_name), Style::default().fg(theme.text)),
        Span::styled(" [Q] Quit [/] Filter [Enter] Inspect [X] Kill [T] Theme ", Style::default().fg(theme.text)),
    ]);
    let header = Paragraph::new(header_text)
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(theme.border)));
    f.render_widget(header, chunks[0]);

    // 2. Top Section
    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[1]);

    // Graphs (Left)
    let graph_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(top_chunks[0]);

    let cpu_data: Vec<u64> = app.cpu_history.iter().cloned().collect();
    f.render_widget(Sparkline::default().block(Block::default().title(" CPU ").borders(Borders::ALL).border_style(Style::default().fg(theme.border))).data(&cpu_data).style(Style::default().fg(theme.graph_cpu)), graph_chunks[0]);

    let mem_data: Vec<u64> = app.mem_history.iter().cloned().collect();
    f.render_widget(Sparkline::default().block(Block::default().title(" Mem ").borders(Borders::ALL).border_style(Style::default().fg(theme.border))).data(&mem_data).style(Style::default().fg(theme.graph_mem)), graph_chunks[1]);

    // Processes List (Right)
    let process_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)]) // Table + Search Bar
        .split(top_chunks[1]);

    let rows: Vec<Row> = app.processes.iter().map(|(pid, name, cpu, mem)| {
        Row::new(vec![
            format!("{}", pid),
            name.clone(),
            format!("{:.1}%", cpu),
            format!("{:.1} MB", *mem as f64 / 1_048_576.0),
        ])
        .style(Style::default().fg(theme.text))
    }).collect();

    let table_title = if app.search_query.is_empty() {
        " Top Processes (Enter to Inspect) ".to_string()
    } else {
        format!(" Search: '{}' ", app.search_query)
    };

    let table = Table::new(rows, [
        Constraint::Length(6), // PID
        Constraint::Percentage(40),
        Constraint::Percentage(25),
        Constraint::Percentage(25),
    ])
    .header(Row::new(vec!["PID", "Name", "CPU", "MEM"]).style(Style::default().fg(theme.border)))
    .block(Block::default().title(table_title).borders(Borders::ALL).border_style(Style::default().fg(theme.border)))
    .row_highlight_style(Style::default().bg(theme.highlight_bg).fg(theme.highlight_fg).add_modifier(Modifier::BOLD));

    f.render_stateful_widget(table, process_chunks[0], &mut app.process_state);

    // Search Input Box
    let input_style = match app.input_mode {
        InputMode::Editing => Style::default().fg(theme.highlight_bg),
        _ => Style::default().fg(Color::DarkGray),
    };
    
    let search_text = if app.input_mode == InputMode::Editing {
        format!("Search: {}_", app.search_query)
    } else {
        format!("Search: {} (Press '/')", app.search_query)
    };

    f.render_widget(Paragraph::new(search_text).style(input_style).block(Block::default().borders(Borders::ALL).title(" Filter ").border_style(Style::default().fg(theme.border))), process_chunks[1]);

    // 3. Gauges
    let gauge_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[2]);

    let cpu_val = *app.cpu_history.back().unwrap_or(&0);
    f.render_widget(Gauge::default().block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(theme.border))).percent(cpu_val as u16).label(format!("CPU: {}%", cpu_val)).gauge_style(Style::default().fg(if cpu_val > 80 { theme.gauge_cpu_high } else { theme.gauge_cpu_low })), gauge_chunks[0]);

    let mem_val = *app.mem_history.back().unwrap_or(&0);
    f.render_widget(Gauge::default().block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(theme.border))).percent(mem_val as u16).label(format!("MEM: {}%", mem_val)).gauge_style(Style::default().fg(theme.gauge_mem)), gauge_chunks[1]);

    // 4. Bottom Section
    let bottom_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[3]);

    // Disk
    let mut disk_rows = Vec::new();
    for disk in &app.disks {
        let total = disk.total_space();
        let available = disk.available_space();
        let used = total - available;
        let percent = if total > 0 { (used as f64 / total as f64 * 100.0) as u16 } else { 0 };
        disk_rows.push(Row::new(vec![
            format!("{:?}", disk.mount_point()),
            format!("{:.1} GB", total as f64 / 1_073_741_824.0),
            format!("{}%", percent),
        ]).style(Style::default().fg(theme.text)));
    }
    f.render_widget(Table::new(disk_rows, [Constraint::Percentage(40), Constraint::Percentage(30), Constraint::Percentage(30)]).block(Block::default().title(" Disks ").borders(Borders::ALL).border_style(Style::default().fg(theme.border))), bottom_chunks[0]);

    // Network Sparklines
    let net_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(bottom_chunks[1]);

    let rx_data: Vec<u64> = app.net_rx_history.iter().cloned().collect();
    f.render_widget(Sparkline::default().block(Block::default().title(" Network RX ").borders(Borders::ALL).border_style(Style::default().fg(theme.border))).data(&rx_data).style(Style::default().fg(theme.graph_net_rx)), net_chunks[0]);

    let tx_data: Vec<u64> = app.net_tx_history.iter().cloned().collect();
    f.render_widget(Sparkline::default().block(Block::default().title(" Network TX ").borders(Borders::ALL).border_style(Style::default().fg(theme.border))).data(&tx_data).style(Style::default().fg(theme.graph_net_tx)), net_chunks[1]);

    // 5. Process Details Popup (Modal)
    if app.input_mode == InputMode::Details {
        if let Some(pid) = app.selected_pid {
            if let Some(process) = app.system.process(pid) {
                let area = centered_rect(60, 50, f.area());
                f.render_widget(Clear, area); // Clear background
                
                let block = Block::default()
                    .title(" Process Details (Esc to Close) ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.border).bg(theme.bg))
                    .style(Style::default().bg(theme.bg));
                f.render_widget(block.clone(), area);

                // Use inner area for content to avoid overlap with borders
                let content_area = block.inner(area);

                let cmd = process.cmd().join(" ");
                let details_text = vec![
                    Line::from(vec![Span::styled("PID: ", Style::default().fg(theme.border)), Span::styled(pid.to_string(), Style::default().fg(theme.text))]),
                    Line::from(vec![Span::styled("Name: ", Style::default().fg(theme.border)), Span::styled(process.name(), Style::default().fg(theme.text))]),
                    Line::from(vec![Span::styled("Status: ", Style::default().fg(theme.border)), Span::styled(format!("{:?}", process.status()), Style::default().fg(theme.text))]),
                    Line::from(vec![Span::styled("CPU Usage: ", Style::default().fg(theme.border)), Span::styled(format!("{:.2}%", process.cpu_usage()), Style::default().fg(theme.text))]),
                    Line::from(vec![Span::styled("Memory: ", Style::default().fg(theme.border)), Span::styled(format!("{:.1} MB", process.memory() as f64 / 1_048_576.0), Style::default().fg(theme.text))]),
                    Line::from(vec![Span::styled("Virtual Mem: ", Style::default().fg(theme.border)), Span::styled(format!("{:.1} MB", process.virtual_memory() as f64 / 1_048_576.0), Style::default().fg(theme.text))]),
                    Line::from(vec![Span::styled("Start Time: ", Style::default().fg(theme.border)), Span::styled(format!("{}s ago", System::uptime().saturating_sub(process.start_time())), Style::default().fg(theme.text))]),
                    Line::from(vec![Span::styled("Disk Read: ", Style::default().fg(theme.border)), Span::styled(format!("{:.1} KB", process.disk_usage().read_bytes as f64 / 1024.0), Style::default().fg(theme.text))]),
                    Line::from(vec![Span::styled("Disk Write: ", Style::default().fg(theme.border)), Span::styled(format!("{:.1} KB", process.disk_usage().written_bytes as f64 / 1024.0), Style::default().fg(theme.text))]),
                    Line::from(""),
                    Line::from(vec![Span::styled("Command: ", Style::default().fg(theme.border))]),
                    Line::from(Span::styled(cmd, Style::default().fg(theme.text))),
                ];

                let p = Paragraph::new(details_text)
                    .wrap(Wrap { trim: true });
                
                f.render_widget(p, content_area);
            }
        }
    }
}

