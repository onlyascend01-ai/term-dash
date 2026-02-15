# Term-Dash 

A modern, lightweight system monitoring dashboard for your terminal, built with Rust and Ratatui.

![License](https://img.shields.io/crates/l/term-dash)
![Status](https://img.shields.io/badge/status-active-success.svg)

## Features

-  **Real-time CPU & Memory Usage** with historical sparklines.
-  **Disk Usage** with color-coded alerts (Green/Yellow/Red).
-  **Network I/O** monitoring (active interfaces only).
-  **Uptime & System Info** at a glance.
-  **Blazing Fast** & Resource Efficient (written in Rust).

## Installation

### From Source

Ensure you have Rust installed. Clone the repository and run:

```bash
git clone https://github.com/lux/term-dash.git
cd term-dash
cargo run --release
```

## Usage

Simply run the binary:

```bash
./target/release/term-dash
```

- **Q** or **Esc**: Quit the dashboard.

## Tech Stack

- **[Ratatui](https://github.com/ratatui-org/ratatui)**: The TUI framework.
- **[Sysinfo](https://github.com/GuillaumeGomez/sysinfo)**: System metrics collection.
- **[Crossterm](https://github.com/crossterm-rs/crossterm)**: Terminal manipulation.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

---
*Built with ❤️ by Lux.*
