use crossterm::event::{KeyCode, KeyEvent};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActiveTab {
    Torrents,
    Peers,
    Logs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TuiCommand {
    Continue,
    Quit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TuiState {
    pub active_tab: ActiveTab,
    pub selected_row: usize,
}

impl Default for TuiState {
    fn default() -> Self {
        Self {
            active_tab: ActiveTab::Torrents,
            selected_row: 0,
        }
    }
}

impl TuiState {
    pub fn handle_key(&mut self, key: KeyEvent, row_count: usize) -> TuiCommand {
        match key.code {
            KeyCode::Char('q') => TuiCommand::Quit,
            KeyCode::Tab => {
                self.active_tab = self.active_tab.next();
                self.selected_row = 0;
                TuiCommand::Continue
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if row_count > 0 {
                    self.selected_row = (self.selected_row + 1).min(row_count - 1);
                }
                TuiCommand::Continue
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selected_row = self.selected_row.saturating_sub(1);
                TuiCommand::Continue
            }
            _ => TuiCommand::Continue,
        }
    }
}

impl ActiveTab {
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Torrents => Self::Peers,
            Self::Peers => Self::Logs,
            Self::Logs => Self::Torrents,
        }
    }

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Torrents => "Torrents",
            Self::Peers => "Peers",
            Self::Logs => "Logs",
        }
    }
}
