use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Clone, Debug, Eq, PartialEq, Parser)]
#[command(name = "styx-cli", about = "Terminal control surface for Styx")]
pub struct Cli {
    #[arg(
        long,
        default_value_t = 6881,
        value_name = "PORT",
        help = "Listen port for torrent transfers"
    )]
    pub listen_port: u16,
    #[arg(
        long,
        help = "Run without rendering the terminal UI and emit JSON lines"
    )]
    pub headless: bool,
    #[arg(
        long,
        value_name = "PATH",
        help = "IPC socket path for control commands"
    )]
    pub ipc: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum Command {
    #[command(about = "Add a .torrent file to the local runtime")]
    Add {
        #[arg(value_name = "TORRENT")]
        source: PathBuf,
        #[arg(long, value_name = "DIR")]
        destination: Option<PathBuf>,
    },
    #[command(about = "Add a magnet URI to the local runtime")]
    AddMagnet {
        #[arg(value_name = "MAGNET_URI")]
        uri: String,
        #[arg(long, value_name = "DIR")]
        destination: PathBuf,
    },
    #[command(about = "Remove a torrent by v1 info hash")]
    Remove {
        #[arg(value_name = "INFO_HASH")]
        info_hash: String,
    },
    #[command(about = "Pause a torrent by v1 info hash")]
    Pause {
        #[arg(value_name = "INFO_HASH")]
        info_hash: String,
    },
    #[command(about = "Resume a torrent by v1 info hash")]
    Resume {
        #[arg(value_name = "INFO_HASH")]
        info_hash: String,
    },
    #[command(about = "Print the current runtime snapshot")]
    Status,
    #[command(about = "Run a real-torrent smoke test that verifies one v1 piece")]
    Smoke {
        #[arg(long, value_name = "TORRENT")]
        torrent: PathBuf,
        #[arg(long, value_name = "DIR")]
        dest: PathBuf,
        #[arg(long, default_value_t = 6881, value_name = "PORT")]
        listen_port: u16,
    },
    #[command(about = "Download a full v1 torrent through the runtime MVP path")]
    Download {
        #[arg(long, value_name = "TORRENT")]
        torrent: PathBuf,
        #[arg(long, value_name = "DIR")]
        dest: PathBuf,
        #[arg(long, default_value_t = 6881, value_name = "PORT")]
        listen_port: u16,
    },
    #[command(subcommand, about = "Run or control the Styx daemon")]
    Daemon(DaemonCommand),
}

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum DaemonCommand {
    #[command(about = "Run the daemon in the foreground")]
    Start {
        #[arg(long, value_name = "DIR")]
        state_dir: PathBuf,
        #[arg(long, value_name = "PATH")]
        socket: PathBuf,
    },
    #[command(about = "Print daemon status over IPC")]
    Status {
        #[arg(long, value_name = "PATH")]
        socket: PathBuf,
    },
    #[command(about = "Stop the daemon over IPC")]
    Stop {
        #[arg(long, value_name = "PATH")]
        socket: PathBuf,
    },
}
