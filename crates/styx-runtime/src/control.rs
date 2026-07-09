use crate::{MagnetAdd, TorrentId, TorrentPlan};

#[derive(Clone, Debug)]
pub enum RuntimeCommand {
    AddPlan(Box<TorrentPlan>),
    AddMagnet(Box<MagnetAdd>),
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
