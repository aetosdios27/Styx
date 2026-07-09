# Styx

A BitTorrent client written in Rust, built around strict protocol handling, verified storage, and private-by-default runtime identity.

Styx is not a wrapper around an existing torrent engine. The core protocol, tracker, disk, peer policy, runtime, CLI, and desktop shell are implemented in this repository.

## Status

Styx is under active development. It can currently run real v1 torrent flows through the runtime path: parse `.torrent` files, announce to trackers, connect to TCP peers, download verified pieces, persist daemon state, and seed verified data back to peers.

Phase 23 work is now underway: Styx has the protocol pieces for DHT/magnet support, a runtime-owned DHT worker primitive, BEP 9 metadata exchange, and an exact-peer magnet resolver. Full app-level DHT-only downloads are still being wired.

| Capability | Status |
|---|---|
| Bencode and metainfo parsing | Implemented |
| Peer-wire handshake and framing | Implemented |
| HTTP and UDP trackers | Implemented |
| Piece/block verification and disk IO | Implemented |
| v1 peer downloads | Implemented |
| Verified seeding/uploading | Implemented |
| Daemon persistence | Implemented |
| CLI, TUI, and headless mode | Implemented |
| Tauri desktop shell | Implemented |
| BitTorrent v2/hybrid groundwork | Implemented |
| DHT runtime worker | Implemented |
| DHT peer source ingestion | Implemented |
| Magnet URI parsing | Implemented |
| BEP 9 metadata exchange | Implemented |
| Exact-peer magnet resolution | Implemented as runtime API |
| DHT-only app/CLI downloads | In progress |
| uTP runtime integration | Planned |
| NAT traversal, encryption, proxy support | Planned |

## Install

Prerequisites:

- Rust toolchain compatible with the workspace `rust-version`
- Cargo
- Bun, for the desktop frontend
- Tauri platform dependencies if running the GUI

Build the workspace:

```sh
cargo build --workspace --locked
```

Run the test suite:

```sh
cargo test --workspace --locked
```

## Usage

Download a legal v1 torrent:

```sh
cargo run -p styx-cli -- download \
  --torrent /absolute/path/file.torrent \
  --dest /tmp/styx-download \
  --listen-port 6881
```

Run a one-piece real-network smoke test:

```sh
cargo run -p styx-cli -- smoke \
  --torrent /absolute/path/file.torrent \
  --dest /tmp/styx-smoke \
  --listen-port 6881
```

Run the daemon:

```sh
cargo run -p styx-cli -- daemon start \
  --state-dir /tmp/styx-state \
  --socket /tmp/styx.sock
```

Query or stop it:

```sh
cargo run -p styx-cli -- daemon status --socket /tmp/styx.sock
cargo run -p styx-cli -- daemon stop --socket /tmp/styx.sock
```

Exact-peer magnet resolution is available as a runtime API for BEP 9 metadata exchange from `x.pe` peers. The CLI/GUI `add-magnet` flow and DHT-backed peer discovery are still in progress.

Run the desktop app:

```sh
cd apps/styx-gui
bun install
bun run app:dev
```

For frontend-only iteration:

```sh
cd apps/styx-gui
bun run dev
```

## Workspace

```text
.
├── apps/
│   ├── styx-cli/       # CLI, TUI, headless mode, daemon commands
│   └── styx-gui/       # Tauri v2 desktop shell
├── crates/
│   ├── styx-app/       # shared app command/snapshot/event contract
│   ├── styx-core/      # peer policy and transfer decisions
│   ├── styx-dht/       # DHT protocol core and UDP socket adapter
│   ├── styx-disk/      # storage, verification, resume
│   ├── styx-ml/        # throttling detection core
│   ├── styx-proto/     # bencode, metainfo, peer protocol, magnets
│   ├── styx-runtime/   # orchestration, daemon, persistence, metadata resolution
│   ├── styx-tracker/   # HTTP/UDP tracker protocol
│   └── styx-utp/       # uTP protocol implementation
└── sim/
    └── swarm-sim/      # swarm simulation experiments
```

## Architecture

```text
apps/styx-cli ─┐
               ├─ styx-app ─ styx-runtime ─┬─ styx-core
apps/styx-gui ─┘                            ├─ styx-disk
                                            ├─ styx-tracker
                                            ├─ styx-dht
                                            └─ styx-proto

styx-dht, styx-utp, and styx-ml are protocol/policy crates that are being integrated progressively into the runtime.
```

## Roadmap

Near-term work:

- Route the daemon-owned DHT worker into active torrents
- Add app/CLI/GUI `add-magnet` command flow
- Persist pending magnet resolution state
- Trackerless torrent startup
- Peer exchange and local service discovery
- Platform packaging

Longer-term work:

- Runtime uTP transport
- NAT traversal
- Protocol encryption and proxy support
- Bandwidth scheduling
- ONNX-backed throttling detection
- Web API and web UI

## License

No license has been declared yet.
