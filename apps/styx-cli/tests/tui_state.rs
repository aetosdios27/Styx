use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use styx_cli::tui::state::{ActiveTab, TuiCommand, TuiState};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn tab_cycles_active_view() {
    let mut state = TuiState::default();

    state.handle_key(key(KeyCode::Tab), 0);

    assert_eq!(state.active_tab, ActiveTab::Peers);
}

#[test]
fn j_and_k_move_selection_within_bounds() {
    let mut state = TuiState::default();

    state.handle_key(key(KeyCode::Char('j')), 2);
    state.handle_key(key(KeyCode::Char('j')), 2);
    state.handle_key(key(KeyCode::Char('j')), 2);
    assert_eq!(state.selected_row, 1);

    state.handle_key(key(KeyCode::Char('k')), 2);
    assert_eq!(state.selected_row, 0);
}

#[test]
fn q_requests_quit() {
    let mut state = TuiState::default();

    let command = state.handle_key(key(KeyCode::Char('q')), 0);

    assert_eq!(command, TuiCommand::Quit);
}
