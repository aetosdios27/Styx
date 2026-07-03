use ratatui::{
    layout::{Constraint, Direction, Layout},
    prelude::Frame,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Sparkline, Table, Tabs},
};
use styx_app::{
    format_bytes, format_percent, format_rate, AppSnapshot, LogLine, PeerRow, TorrentRow,
};

use crate::tui::state::{ActiveTab, TuiState};
pub fn render(frame: &mut Frame<'_>, snapshot: &AppSnapshot, state: &TuiState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(6),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let tabs = Tabs::new(["Torrents", "Peers", "Logs"])
        .select(match state.active_tab {
            ActiveTab::Torrents => 0,
            ActiveTab::Peers => 1,
            ActiveTab::Logs => 2,
        })
        .block(Block::default().title("Styx").borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));
    frame.render_widget(tabs, chunks[0]);

    match state.active_tab {
        ActiveTab::Torrents => render_torrents(frame, chunks[1], &snapshot.torrents),
        ActiveTab::Peers => render_peers(frame, chunks[1], &snapshot.peers),
        ActiveTab::Logs => render_logs(frame, chunks[1], &snapshot.logs),
    }

    let speed_values = snapshot
        .speed
        .iter()
        .map(|sample| sample.down_rate)
        .collect::<Vec<_>>();
    let footer = Sparkline::default()
        .block(Block::default().title("Down speed").borders(Borders::ALL))
        .data(&speed_values);
    frame.render_widget(footer, chunks[2]);
}

fn render_torrents(frame: &mut Frame<'_>, area: ratatui::layout::Rect, torrents: &[TorrentRow]) {
    let rows = torrents.iter().map(|torrent| {
        Row::new([
            Cell::from(torrent.name.clone()),
            Cell::from(format_bytes(torrent.size_bytes)),
            Cell::from(format_percent(torrent.progress)),
            Cell::from(format_rate(torrent.down_rate)),
            Cell::from(format_rate(torrent.up_rate)),
            Cell::from(format!("{}/{}", torrent.seeds, torrent.peers)),
            Cell::from(format!("{:?}", torrent.status).to_lowercase()),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(28),
            Constraint::Percentage(12),
            Constraint::Percentage(12),
            Constraint::Percentage(14),
            Constraint::Percentage(14),
            Constraint::Percentage(10),
            Constraint::Percentage(10),
        ],
    )
    .header(Row::new([
        "Name", "Size", "Progress", "Down", "Up", "S/P", "State",
    ]))
    .block(Block::default().title("Torrents").borders(Borders::ALL));
    frame.render_widget(table, area);
}

fn render_peers(frame: &mut Frame<'_>, area: ratatui::layout::Rect, peers: &[PeerRow]) {
    let rows = peers.iter().map(|peer| {
        Row::new([
            Cell::from(peer.address.clone()),
            Cell::from(peer.flags.clone()),
            Cell::from(format_percent(peer.progress)),
            Cell::from(format_rate(peer.down_rate)),
            Cell::from(format_rate(peer.up_rate)),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(35),
            Constraint::Percentage(10),
            Constraint::Percentage(15),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
        ],
    )
    .header(Row::new(["Peer", "Flags", "Progress", "Down", "Up"]))
    .block(Block::default().title("Peers").borders(Borders::ALL));
    frame.render_widget(table, area);
}

fn render_logs(frame: &mut Frame<'_>, area: ratatui::layout::Rect, logs: &[LogLine]) {
    let lines = if logs.is_empty() {
        vec![Line::from("No log entries")]
    } else {
        logs.iter()
            .map(|line| {
                Line::from(vec![
                    Span::raw(format!("{:?}", line.level).to_lowercase()),
                    Span::raw(" "),
                    Span::raw(line.message.clone()),
                ])
            })
            .collect()
    };
    let paragraph =
        Paragraph::new(lines).block(Block::default().title("Logs").borders(Borders::ALL));
    frame.render_widget(paragraph, area);
}
