# Styx

Styx is a BitTorrent client being built around a simple premise: a torrent client should be correct, secure, and private by construction, not by accident.

The project is a Rust-first implementation of the BitTorrent stack, from wire formats and disk verification up through a daemon, terminal UI, and Tauri desktop shell. The long-term target is a modern client with BitTorrent v2 and hybrid torrent support, ephemeral identity, verified storage, resilient peer orchestration, and adaptive throttling detection.

This repository is not a skin over an existing torrent engine. Styx implements the protocol layers directly.

## Current Status

Styx can already parse and validate torrents, talk to trackers, connect to peers, download verified v1 pieces, complete full v1 downloads through the runtime path, persist daemon state, and seed verified data back to peers.

The next major implementation target is trackerless startup through DHT bootstrap and magnet links.

| Area | Status |
|---|---|
| Bencode and metainfo parsing | Implemented |
| Peer-wire handshake and message framing | Implemented |
| HTTP and UDP trackers | Implemented |
| Piece/block management and disk verification | Implemented |
| v1 peer downloads | Implemented |
| Verified seeding/upload serving | Implemented |
| Runtime orchestration | Implemented |
| Staged mutation pipeline | Implemented |
| Daemon state persistence | Implemented |
| CLI, TUI, headless JSONL mode | Implemented |
| Tauri desktop shell | Implemented |
| BitTorrent v2 and hybrid metadata/storage groundwork | Implemented |
| DHT protocol core | Implemented |
| uTP protocol core | Implemented |
| ML throttling-detection core | Implemented |
| DHT bootstrap into runtime | In progress next |
| Magnet metadata exchange | In progress next |
| NAT traversal, protocol encryption, proxy support, scheduling | Planned |

## Why Styx Exists

Most torrent clients evolved over many years by accreting protocol features, UI features, and compatibility fixes. That is practical, but it often makes correctness and privacy secondary properties.

Styx takes the opposite route:

- Parse untrusted bytes strictly before they touch runtime state.
- Verify downloaded data before exposing it as complete.
- Keep peer policy deterministic and testable away from socket IO.
- Treat user actions as staged intents: declare, validate, execute, rollback.
- Avoid persistent peer identity by default.
- Build v2 and hybrid torrent support into the architecture instead of bolting it on later.
- Keep CLI, GUI, and daemon surfaces on one shared app/runtime contract.

The project is intentionally protocol-heavy. The point is not only to make a client that works, but to make one whose behavior can be audited.

## Architecture

```text
              apps/styx-cli          apps/styx-gui
                    │                     │
                    └───────── styx-app ──┘
                              │
                         styx-runtime
                              │
        ┌───────────────┬─────┴─────┬───────────────┐
        │               │           │               │
    styx-core       styx-disk   styx-tracker     styx-dht
        │               │           │               │
        └───────────────┴─────┬─────┴───────────────┘
                              │
                         styx-proto
                              │
                          swarm-sim

Separate transport work:

    styx-utp      BEP 29 uTP packet, connection, LEDBAT, socket smoke layer
    styx-ml       privacy-safe throttling feature extraction and policy core
```

### Crates

| Crate | Responsibility |
|---|---|
| `styx-proto` | Bencode, torrent metainfo, peer-wire protocol, v1/v2 hash metadata primitives |
| `styx-tracker` | HTTP tracker, UDP tracker, multitracker policy, compact peer parsing |
| `styx-disk` | Piece layout, block assembly, hash verification, file mapping, resume checks |
| `styx-core` | Socket-free peer policy: choking, request scheduling, rarest-first, endgame, upload serving |
| `styx-runtime` | Real torrent orchestration, peer IO, source handling, daemon, persistence, staged intents |
| `styx-app` | Shared command/snapshot/event contract used by CLI and GUI |
| `styx-cli` | Terminal UI, headless JSONL mode, daemon IPC commands, smoke/download commands |
| `styx-gui` | Tauri v2 + React + Vite + Tailwind desktop shell |
| `styx-dht` | BEP 5 KRPC, routing table, tokens, peer store, BEP 42 identity, IPv6 groundwork |
| `styx-utp` | BEP 29 uTP transport mechanics and LEDBAT behavior |
| `styx-ml` | Model-independent throttling feature extraction, normalization, and policy mapping |
| `swarm-sim` | Discrete-event swarm simulator for rarest-first and piece-selection experiments |

## What Works Today

### Runtime downloads

The runtime can load legal v1 `.torrent` files, discover sources through trackers and web seeds, connect to TCP peers, request pieces through the peer-wire protocol, verify blocks through the disk layer, and transition completed torrents into seeding mode.

### Seeding

Styx serves only verified data. Upload requests are routed through the peer policy layer, then fulfilled from `styx-disk` only if the requested block belongs to a verified piece.

Completed torrents restore as seedable after daemon restart only after disk data is re-verified.

### Daemon and control surfaces

The CLI can run as:

- an interactive terminal UI,
- a headless JSONL process,
- a daemon controller over IPC,
- a direct smoke/download runner for real torrent tests.

The GUI is a Tauri v2 desktop shell over the same app contract, not a separate product path.

### Protocol foundations

Implemented protocol pieces include:

- BEP 3 bencode and v1 metainfo parsing
- BEP 3 peer-wire handshake and core messages
- BEP 15 UDP tracker packets and state machine
- BEP 19-style web seed range validation in the runtime
- BEP 29 uTP packet/connection behavior
- BEP 42 DHT node identity groundwork
- BEP 52 v2/hybrid metadata and Merkle verification groundwork

## What Is Not Done Yet

Styx is not yet a drop-in replacement for qBittorrent, Transmission, or libtorrent-backed clients.

Important missing or incomplete areas:

- Magnet links do not yet drive end-to-end metadata exchange.
- Runtime DHT bootstrap is the next implementation phase.
- uTP exists as a protocol crate, but is not yet the primary runtime transport.
- Full production NAT traversal is not implemented.
- MSE/PE protocol encryption and proxy support are planned.
- The ONNX-backed ML adapter is planned; the current ML crate owns the model-independent feature and policy core.
- Distribution packaging is not complete.

## Quick Start

### Prerequisites

- Rust toolchain compatible with the workspace `rust-version`
- Cargo
- Bun, for the GUI frontend
- Platform dependencies required by Tauri v2 if building the desktop app

### Build

```sh
cargo build --workspace --locked
```

### Test

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
```

### Run a full v1 torrent download

Use only torrents you are legally allowed to download.

```sh
cargo run -p styx-cli -- download \
  --torrent /absolute/path/legal-v1.torrent \
  --dest /tmp/styx-download \
  --listen-port 6881
```

### Run a one-piece smoke test

```sh
cargo run -p styx-cli -- smoke \
  --torrent /absolute/path/legal-v1.torrent \
  --dest /tmp/styx-smoke \
  --listen-port 6881
```

### Run the daemon

```sh
cargo run -p styx-cli -- daemon start \
  --state-dir /tmp/styx-state \
  --socket /tmp/styx.sock
```

In another terminal:

```sh
cargo run -p styx-cli -- daemon status --socket /tmp/styx.sock
cargo run -p styx-cli -- daemon stop --socket /tmp/styx.sock
```

### Run the desktop app in development

```sh
cd apps/styx-gui
bun install
bun run app:dev
```

For frontend-only work:

```sh
cd apps/styx-gui
bun run dev
```

The Tauri command opens the desktop app. The Vite command opens the frontend in a browser and is useful for UI iteration, but it is not the full desktop runtime.

## Engineering Principles

### Strict protocol boundaries

Untrusted network and torrent data is parsed into typed structures before runtime code acts on it. Malformed inputs should produce typed errors, not panics, silent truncation, or implicit defaults.

### Verification before trust

Downloaded data is not considered complete until the disk layer verifies it. Seeding reads only verified blocks.

### Socket-free policy

Peer scheduling, choking, upload serving decisions, and rarest-first selection live in deterministic policy code. Runtime IO executes effects emitted by that policy.

### Staged intents

Mutating operations are modeled as staged intents:

```text
declare intent -> validate constraints -> execute -> commit or rollback
```

This is used to keep app, daemon, and runtime state transitions explicit.

### Privacy by default

The architecture avoids long-lived local peer identity by default. Peer IDs and DHT identity are treated as runtime identity, not durable user identity.

### Test the unhappy path

The project includes deterministic tests for broken trackers, corrupt peers, slow peers, bad web seeds, disk pressure, resume behavior, daemon restart recovery, and upload serving.

## Verification Culture

The normal baseline is:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
```

Focused crate-level commands are preferred while developing a slice, then the full baseline runs before claiming completion.

Ignored tests are reserved for real socket or real-network boundaries and must be run explicitly.

## Current Roadmap Direction

Near-term:

1. DHT bootstrap owned by the daemon/runtime.
2. Magnet URI parsing and BEP 9 metadata exchange.
3. Trackerless torrent startup through DHT-discovered peers.
4. Peer exchange and local service discovery as source enrichments.
5. Platform polish and distribution.

Longer-term:

- Full runtime integration for uTP.
- NAT traversal and connection infrastructure.
- Protocol encryption and proxy support.
- Bandwidth scheduling.
- ONNX-backed throttling detection.
- Web API and web UI.
- Distribution packaging across major platforms.
- Advanced BEPs and observability.

## Repository Layout

```text
.
├── apps/
│   ├── styx-cli/       # CLI, TUI, headless mode, daemon commands
│   └── styx-gui/       # Tauri v2 desktop shell
├── crates/
│   ├── styx-app/       # app-facing command/snapshot/event contract
│   ├── styx-core/      # peer policy and transfer decisions
│   ├── styx-dht/       # DHT protocol/runtime core
│   ├── styx-disk/      # storage, verification, resume
│   ├── styx-ml/        # throttling detection core
│   ├── styx-proto/     # bencode, metainfo, peer protocol
│   ├── styx-runtime/   # orchestration, daemon, persistence
│   ├── styx-tracker/   # HTTP/UDP tracker protocol
│   └── styx-utp/       # uTP protocol implementation
└── sim/
    └── swarm-sim/      # swarm simulation experiments
```

## Legal and Safety Notes

Styx is a protocol implementation. Use it only with content you have the right to download, upload, seed, or test.

The project intentionally includes real-network smoke hooks, but those should be pointed only at legal/public-domain or otherwise authorized torrents.

## License

No license has been declared yet.

