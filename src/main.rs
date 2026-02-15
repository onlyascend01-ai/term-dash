use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Row, Sparkline, Table, TableState},
    Terminal,
};
use std::{collections::VecDeque, io, time::{Duration, Instant}};
use sysinfo::{
    CpuRefreshKind, Disks, MemoryRefreshKind, Networks, ProcessRefreshKind, RefreshKind, System, Pid,
};

const TICK_RATE: u64 = 1000;
const HISTORY_LEN: usize = 100;

struct App {
    system: System,
    networks: Networks,
    disks: Disks,
    cpu_history: VecDeque<u64>,
    mem_history: VecDeque<u64>,
    should_quit: bool,
    // Process Interaction
    process_state: TableState,
    processes: Vec<(Pid, String, f32, u64)>, // Cache for stable indexing
    input_mode: InputMode,
    search_query: String,
}

#[derive(Clone, Copy, PartialEq)]
enum InputMode {
    Normal,
    Editing,
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
            should_quit: false,
            process_state,
            processes: Vec::new(),
            input_mode: InputMode::Normal,
            search_query: String::new(),
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

        // Update Process Cache
        let mut procs: Vec<_> = self.system.processes().values().collect();
        
        // Filter if searching
        if !self.search_query.is_empty() {
            procs.retain(|p| p.name().to_lowercase().contains(&self.search_query.to_lowercase()));
            // Sort filtered results by name for stability, or CPU if preferred
            procs.sort_by(|a, b| b.cpu_usage().partial_cmp(&a.cpu_usage()).unwrap_or(std::cmp::Ordering::Equal));
        } else {
            // Default: Top 20 by CPU
            procs.sort_by(|a, b| b.cpu_usage().partial_cmp(&a.cpu_usage()).unwrap_or(std::cmp::Ordering::Equal));
            procs.truncate(20);
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
            if let Some((pid, name, _, _)) = self.processes.get(i) {
                // Attempt to kill
                if let Some(process) = self.system.process(*pid) {
                    process.kill();
                }
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
                                app.process_state.select(Some(0)); // Reset selection
                            }
                            _ => {}
                        },
                        InputMode::Editing => match key.code {
                            KeyCode::Enter => {
                                app.input_mode = InputMode::Normal;
                            }
                            KeyCode::Esc => {
                                app.input_mode = InputMode::Normal;
                                app.search_query.clear();
                            }
                            KeyCode::Backspace => {
                                app.search_query.pop();
                            }
                            KeyCode::Char(c) => {
                                app.search_query.push(c);
                            }
                            _ => {}
                        },
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

fn ui(f: &mut ratatui::Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),      // Header
            Constraint::Percentage(40), // Top: Graphs + Processes
            Constraint::Percentage(20), // Gauges
            Constraint::Percentage(40), // Bottom: Disk + Net
        ])
        .split(f.area());

    // 1. Header
    let host_name = System::host_name().unwrap_or_else(|| "Unknown".to_string());
    let header_text = Line::from(vec![
        Span::styled(" TERM-DASH v0.3 ", Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(format!(" | Host: {} ", host_name)),
        Span::styled(" [Q] Quit [/] Filter [Up/Down] Select [X] Kill ", Style::default().fg(Color::Yellow)),
    ]);
    let header = Paragraph::new(header_text)
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Cyan)));
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
    f.render_widget(Sparkline::default().block(Block::default().title(" CPU ").borders(Borders::ALL)).data(&cpu_data).style(Style::default().fg(Color::Green)), graph_chunks[0]);

    let mem_data: Vec<u64> = app.mem_history.iter().cloned().collect();
    f.render_widget(Sparkline::default().block(Block::default().title(" Mem ").borders(Borders::ALL)).data(&mem_data).style(Style::default().fg(Color::Magenta)), graph_chunks[1]);

    // Processes List (Right) - With Search!
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
    }).collect();

    let table_title = if app.search_query.is_empty() {
        " Top Processes (CPU) ".to_string()
    } else {
        format!(" Search: '{}' ", app.search_query)
    };

    let table = Table::new(rows, [
        Constraint::Length(6), // PID
        Constraint::Percentage(40),
        Constraint::Percentage(25),
        Constraint::Percentage(25),
    ])
    .header(Row::new(vec!["PID", "Name", "CPU", "MEM"]).style(Style::default().fg(Color::Yellow)))
    .block(Block::default().title(table_title).borders(Borders::ALL))
    .highlight_style(Style::default().bg(Color::Red).fg(Color::White).add_modifier(Modifier::BOLD));

    f.render_stateful_widget(table, process_chunks[0], &mut app.process_state);

    // Search Input Box
    let input_style = match app.input_mode {
        InputMode::Normal => Style::default().fg(Color::DarkGray),
        InputMode::Editing => Style::default().fg(Color::Yellow),
    };
    
    let search_text = if app.input_mode == InputMode::Editing {
        format!("Search: {}_", app.search_query) // Cursor
    } else {
        format!("Search: {} (Press '/')", app.search_query)
    };

    let search_bar = Paragraph::new(search_text)
        .style(input_style)
        .block(Block::default().borders(Borders::ALL).title(" Filter "));
    
    f.render_widget(search_bar, process_chunks[1]);

    // 3. Gauges
    let gauge_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[2]);

    let cpu_val = *app.cpu_history.back().unwrap_or(&0);
    f.render_widget(Gauge::default().block(Block::default().borders(Borders::ALL)).percent(cpu_val as u16).label(format!("CPU: {}%", cpu_val)).gauge_style(Style::default().fg(if cpu_val > 80 { Color::Red } else { Color::Green })), gauge_chunks[0]);

    let mem_val = *app.mem_history.back().unwrap_or(&0);
    f.render_widget(Gauge::default().block(Block::default().borders(Borders::ALL)).percent(mem_val as u16).label(format!("MEM: {}%", mem_val)).gauge_style(Style::default().fg(Color::Magenta)), gauge_chunks[1]);

    // 4. Bottom Section
    let bottom_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
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
        ]));
    }
    f.render_widget(Table::new(disk_rows, [Constraint::Percentage(40), Constraint::Percentage(30), Constraint::Percentage(30)]).block(Block::default().title(" Disks ").borders(Borders::ALL)), bottom_chunks[0]);

    // Network
    let mut net_rows = Vec::new();
    for (name, data) in &app.networks {
        if data.received() > 0 || data.transmitted() > 0 {
            net_rows.push(Row::new(vec![
                name.clone(),
                format!("{:.1} KB", data.received() as f64 / 1024.0),
                format!("{:.1} KB", data.transmitted() as f64 / 1024.0),
            ]));
        }
    }
    f.render_widget(Table::new(net_rows, [Constraint::Percentage(40), Constraint::Percentage(30), Constraint::Percentage(30)]).block(Block::default().title(" Network ").borders(Borders::ALL)), bottom_chunks[1]);
}
