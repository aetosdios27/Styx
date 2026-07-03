pub mod state;
pub mod view;

use std::time::Duration;

use crossterm::{
    event::{self, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::{error::CliError, runtime::TorrentRuntime};

use self::state::{ActiveTab, TuiCommand, TuiState};

pub async fn run_tui<R>(mut runtime: R) -> Result<(), CliError>
where
    R: TorrentRuntime,
{
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let mut state = TuiState::default();

    let result = loop {
        let snapshot = runtime.snapshot();
        terminal.draw(|frame| view::render(frame, &snapshot, &state))?;

        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                let rows = match state.active_tab {
                    ActiveTab::Torrents => snapshot.torrents.len(),
                    ActiveTab::Peers => snapshot.peers.len(),
                    ActiveTab::Logs => snapshot.logs.len(),
                };
                if state.handle_key(key, rows) == TuiCommand::Quit {
                    break Ok(());
                }
            }
        }
        let _ = runtime.tick();
    };

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}
