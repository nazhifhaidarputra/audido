use std::fs::canonicalize;
use std::io::IsTerminal;
#[cfg(target_os = "linux")]
use std::process::Command;
use std::time::Duration;
use std::{io, path::PathBuf};

use audido_core::browser;
use audido_core::commands::CoreEvent;
use ratatui::{
    backend::CrosstermBackend,
    crossterm::{
        event::{self, Event, KeyCode, KeyEventKind},
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    },
};

use audido_core::modules::{self, core::CoreHandle};

mod logger;
mod router;
mod routes;
mod state;
mod states;
mod ui;

use router::{Router, route_for_name, tab_names};
use state::AppState;

use crate::router::InterceptKeyResult;
use crate::routes::playback::PlaybackRoute;

fn main() -> anyhow::Result<()> {
    if !std::io::stdout().is_terminal() {
        let exe = std::env::current_exe()?;
        let args: Vec<String> = std::env::args().skip(1).collect();

        #[cfg(target_os = "windows")]
        Command::new("cmd").arg("/c").arg("start").arg("").arg(&exe).args(&args).spawn()?;

        #[cfg(target_os = "macos")]
        Command::new("open").arg("-a").arg("Terminal").arg(&exe).args(&args).spawn()?;

        #[cfg(target_os = "linux")]
        {
            let terminals = [
                ("kitty", "-e"),
                ("alacritty", "-e"),
                ("x-terminal-emulator", "-e"),
                ("gnome-terminal", "--"),
                ("konsole", "-e"),
                ("xfce4-terminal", "-e"),
                ("mate-terminal", "-e"),
                ("terminator", "-e"),
                ("xterm", "-e"),
            ];

            let mut spawned = false;

            while let Some(&(term, flag)) = terminals.iter().next() {
                if Command::new(term).arg(flag).arg(&exe).args(&args).spawn().is_ok() {
                    spawned = true;
                    break;
                }
            }

            if !spawned {
                anyhow::bail!("Failed to launch any supported Linux terminal emulator.");
            }
        }

        // Exit the initial invisible process so the new terminal takes over
        return Ok(());
    }
    logger::setup_logging()?;

    log::info!("Starting Audido TUI");

    // Get audio file paths from command line args
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Initialise the new CPAL-based audio core
    let handle = audido_core::modules::core::init()?;

    // Ensure clean shutdown
    run_tui(handle, args)
}

fn run_tui(mut handle: CoreHandle, initial_files: Vec<String>) -> anyhow::Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend)?;

    let mut state = AppState::new();
    let mut router = Router::new(Box::new(PlaybackRoute));

    // Subscribe to broadcast events from the audio core
    let mut event_rx = handle.subscribe();

    // Handle initial setup (Browser context & Queue loading)
    setup_initial_state(&mut state, &handle, initial_files)?;

    loop {
        // Drain all pending CoreEvents from the broadcast channel
        loop {
            match event_rx.try_recv() {
                Ok(event) => {
                    // Trigger the recovery mechanism when the device invalidates
                    if matches!(event, CoreEvent::DeviceInvalidated) {
                        log::info!("Device invalidated event received, attempting host recovery...");
                        if let Err(e) = audido_core::modules::core::resolve_host(&mut handle) {
                            log::error!("Host recovery failed: {}", e);
                        }
                    }
                    
                    state.handle_event(event);
                },
                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
                Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                    log::warn!("Core event channel closed");
                    break;
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(n)) => {
                    log::warn!("TUI lagged behind {} core events", n);
                }
            }
        }

        // Draw UI
        terminal.draw(|f| ui::draw(f, &state, &router))?;

        // Handle input
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            // check if current route wants to intercept the key
            match router
                .current_mut()
                .intercept_global_key(key.code, &mut state, &handle)
            {
                InterceptKeyResult::Handled => {
                    continue;
                }
                InterceptKeyResult::HandledAndNavigate(action) => {
                    router.execute_action(action, &mut state, &handle)?;
                }
                InterceptKeyResult::Ignored => {
                    // Handle global keys first
                    let should_quit =
                        handle_global_keys(key.code, &mut state, &handle, &mut router)?;
                    if should_quit {
                        break;
                    }
                }
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}

fn setup_initial_state(
    state: &mut AppState,
    handle: &CoreHandle,
    files: Vec<String>,
) -> anyhow::Result<()> {
    if files.is_empty() {
        return Ok(());
    }

    // Set Browser Context based on the first file
    if let Some(first_file) = files.first() {
        let path = PathBuf::from(first_file);

        let target_dir = if let Ok(abs_path) = canonicalize(&path) {
            if abs_path.is_dir() {
                Some(abs_path)
            } else {
                abs_path.parent().map(|p| p.to_path_buf())
            }
        } else if path.is_dir() {
            Some(path)
        } else {
            path.parent().map(|p| p.to_path_buf())
        };

        if let Some(dir) = target_dir
            && let Ok(items) = browser::get_directory_content(&dir)
        {
            state.browser.current_dir = dir;
            state.browser.items = items;
            state.browser.list_state.select(Some(0));
            log::info!("Browser context set to: {:?}", state.browser.current_dir);
        }
    }

    log::info!("Adding {} files to queue from CLI", files.len());
    // Non-blocking: queue is populated by Tokio background task
    
    let ctx = handle.ctx();
    let tokio_handle = handle.ctx().tokio_handle.clone();

    tokio_handle.spawn(async move {
        modules::queue::add_to_queue(ctx, files).await;
    });
    state.audio.status_message = "Loading queue...".to_string();

    Ok(())
}

/// Handle global keys and delegate route-specific input to router
fn handle_global_keys(
    key: KeyCode,
    state: &mut AppState,
    handle: &CoreHandle,
    router: &mut Router,
) -> anyhow::Result<bool> {
    // Global keys that work regardless of route
    match key {
        KeyCode::Char('q') => {
            // Graceful shutdown — stop DSP task and let CPAL stream drop naturally
            modules::playback::stop(handle.ctx());
            return Ok(true);
        }
        KeyCode::Tab => {
            // Cycle through tabs
            let tabs = tab_names();
            let current_name = router.current().name();
            let current_idx = tabs.iter().position(|n| *n == current_name).unwrap_or(0);
            let next_idx = (current_idx + 1) % tabs.len();
            let next_route = route_for_name(tabs[next_idx]);
            router.replace(next_route, state, handle)?;
            return Ok(false);
        }
        KeyCode::Esc => {
            // Try to pop from router (go back)
            if router.depth() > 1 {
                router.pop(state, handle)?;
                return Ok(false);
            }
        }
        _ => {}
    }

    // Delegate to the current route's input handler
    let action = router.current_mut().handle_input(key, state, handle)?;
    let should_quit = router.execute_action(action, state, handle)?;
    Ok(should_quit)
}
