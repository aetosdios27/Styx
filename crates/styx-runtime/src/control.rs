use crate::{TorrentId, TorrentPlan};

#[derive(Clone, Debug)]
pub enum RuntimeCommand {
    AddPlan(Box<TorrentPlan>),
    Torrent(TorrentId, TorrentCommand),
    Remove(TorrentId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TorrentCommand {
    Start,
    Pause,
    Resume,
    Cancel,
    Tick,
}
