use ratatui::{backend::TestBackend, Terminal};
use styx_cli::{
    format::InfoHashHex,
    model::{AppSnapshot, LogLevel, LogLine, PeerRow, TorrentRow, TorrentStatus},
    tui::{
        state::{ActiveTab, TuiState},
        view::render,
    },
};

#[test]
fn torrent_tab_renders_torrent_name() {
    let backend = TestBackend::new(90, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let snapshot = AppSnapshot {
        torrents: vec![TorrentRow {
            info_hash: InfoHashHex::repeat(0x11),
            name: "ubuntu.iso".to_owned(),
            status: TorrentStatus::Checking,
            size_bytes: 1024,
            progress: 0.25,
            down_rate: 2048,
            up_rate: 0,
            peers: 2,
            seeds: 1,
        }],
        ..AppSnapshot::default()
    };

    terminal
        .draw(|frame| render(frame, &snapshot, &TuiState::default()))
        .unwrap();

    assert!(buffer_text(terminal.backend()).contains("ubuntu.iso"));
}

#[test]
fn peer_tab_renders_peer_address() {
    let backend = TestBackend::new(90, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let snapshot = AppSnapshot {
        peers: vec![PeerRow {
            torrent: InfoHashHex::repeat(0x11),
            address: "127.0.0.1:6881".to_owned(),
            flags: "I".to_owned(),
            progress: 0.5,
            down_rate: 10,
            up_rate: 5,
        }],
        ..AppSnapshot::default()
    };
    let state = TuiState {
        active_tab: ActiveTab::Peers,
        ..TuiState::default()
    };

    terminal
        .draw(|frame| render(frame, &snapshot, &state))
        .unwrap();

    assert!(buffer_text(terminal.backend()).contains("127.0.0.1:6881"));
}

#[test]
fn logs_tab_renders_log_message() {
    let backend = TestBackend::new(90, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let snapshot = AppSnapshot {
        logs: vec![LogLine {
            level: LogLevel::Info,
            message: "daemon ready".to_owned(),
        }],
        ..AppSnapshot::default()
    };
    let state = TuiState {
        active_tab: ActiveTab::Logs,
        ..TuiState::default()
    };

    terminal
        .draw(|frame| render(frame, &snapshot, &state))
        .unwrap();

    assert!(buffer_text(terminal.backend()).contains("daemon ready"));
}

fn buffer_text(backend: &TestBackend) -> String {
    let area = backend.buffer().area;
    let mut text = String::new();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            text.push_str(backend.buffer().cell((x, y)).unwrap().symbol());
        }
        text.push('\n');
    }
    text
}
