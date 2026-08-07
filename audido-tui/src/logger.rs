use log::{Level, LevelFilter};
use ratatui::style::{Color, Style};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

// Define a struct to hold log data
#[derive(Clone)]
pub struct LogRecord {
    pub level: Level,
    pub message: String,
    pub timestamp: String,
}

// Global shared buffer for the TUI to read
// We use a Mutex so both the Logger (background) and UI (main thread) can access it
lazy_static::lazy_static! {
    pub static ref LOG_BUFFER: Arc<Mutex<VecDeque<LogRecord>>> = Arc::new(Mutex::new(VecDeque::with_capacity(1000)));
}

pub fn setup_logging() -> anyhow::Result<()> {
    let _ = std::fs::remove_file("audido.log");

    fern::Dispatch::new()
        .format(|out, message, record| {
            let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();

            // Format for the file
            out.finish(format_args!(
                "[{}][{}] {}",
                timestamp,
                record.level(),
                message
            ))
        })
        .level(LevelFilter::Debug)
        .chain(fern::log_file("audido.log")?)
        .chain(fern::Output::call(|record| {
            let log_record = LogRecord {
                level: record.level(),
                message: record.args().to_string(),
                timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
            };

            if let Ok(mut buffer) = LOG_BUFFER.lock() {
                if buffer.len() >= 1000 {
                    buffer.pop_front(); // Keep buffer size manageable
                }
                buffer.push_back(log_record);
            }
        }))
        .apply()?;

    Ok(())
}

// Helper to get color based on level
pub fn get_level_style(level: Level) -> Style {
    match level {
        Level::Error => Style::default().fg(Color::Red),
        Level::Warn => Style::default().fg(Color::Yellow),
        Level::Info => Style::default().fg(Color::Cyan),
        Level::Debug => Style::default().fg(Color::Green),
        Level::Trace => Style::default().fg(Color::Magenta),
    }
}
