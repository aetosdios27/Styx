use std::collections::{BTreeSet, HashMap};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use arrow_array::{BooleanArray, Float64Array, RecordBatch, StringArray, UInt64Array};
use arrow_schema::{DataType, Field, Schema};
use clap::Parser;
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use rand::distributions::{Distribution, Uniform, WeightedIndex};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rand_distr::{Exp, LogNormal};
use serde::{Deserialize, Serialize};
use tracing::info;

const BLOCK_BYTES: u64 = 16 * 1024;
const SNAPSHOT_INTERVAL_TICKS: u64 = 60;

#[derive(Debug, Parser)]
#[command(about = "Generate synthetic BitTorrent swarm training data")]
struct Args {
    #[arg(long)]
    out: PathBuf,
    #[arg(long, default_value_t = 42)]
    seed: u64,
    #[arg(long, default_value_t = 10_000)]
    peers: usize,
    #[arg(long, default_value_t = 2_048)]
    pieces: usize,
    #[arg(long, default_value_t = 86_400)]
    duration_secs: u64,
    #[arg(long, default_value_t = 10)]
    tick_secs: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum PeerRole {
    Seeder,
    Leecher,
}

impl PeerRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::Seeder => "seeder",
            Self::Leecher => "leecher",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum ClientFamily {
    QBittorrent,
    Transmission,
    UTorrent,
    Libtorrent,
    Unknown,
}

impl ClientFamily {
    fn as_str(self) -> &'static str {
        match self {
            Self::QBittorrent => "qbittorrent",
            Self::Transmission => "transmission",
            Self::UTorrent => "utorrent",
            Self::Libtorrent => "libtorrent",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug)]
struct Peer {
    id: u64,
    role: PeerRole,
    client: ClientFamily,
    start_tick: u64,
    end_tick: u64,
    upload_rate_bps: f64,
    latency_ms: f64,
    reliability: f64,
    churn_tendency: f64,
    pieces: BTreeSet<usize>,
    uploaded_bytes: u64,
    delivered_blocks: u64,
    failed_blocks: u64,
    latency_sum_ms: f64,
    first_tick: Option<u64>,
    last_tick: Option<u64>,
}

impl Peer {
    fn active_at(&self, tick: u64) -> bool {
        self.start_tick <= tick && tick < self.end_tick
    }

    fn bitfield_density(&self, pieces: usize) -> f64 {
        self.pieces.len() as f64 / pieces as f64
    }

    fn quality_label(&self) -> f64 {
        let total = self.delivered_blocks + self.failed_blocks;
        if total == 0 {
            return (self.reliability * 0.5).clamp(0.0, 1.0);
        }

        let success_rate = self.delivered_blocks as f64 / total as f64;
        let latency_penalty = if self.delivered_blocks == 0 {
            0.0
        } else {
            let mean_latency = self.latency_sum_ms / self.delivered_blocks as f64;
            (1.0 - (mean_latency / 2_000.0)).clamp(0.0, 1.0)
        };
        let throughput = (self.uploaded_bytes as f64 / 50_000_000.0).clamp(0.0, 1.0);

        (success_rate * 0.55 + latency_penalty * 0.25 + throughput * 0.20).clamp(0.0, 1.0)
    }
}

#[derive(Debug)]
struct SimConfig {
    out: PathBuf,
    seed: u64,
    peers: usize,
    pieces: usize,
    duration_secs: u64,
    tick_secs: u64,
}

#[derive(Default, Debug)]
struct SimulationOutput {
    peer_sessions: Vec<PeerSessionRow>,
    block_events: Vec<BlockEventRow>,
    seeder_events: Vec<SeederEventRow>,
    piece_snapshots: Vec<PieceSnapshotRow>,
}

#[derive(Debug)]
struct PeerSessionRow {
    peer_id: u64,
    role: PeerRole,
    client_id: ClientFamily,
    session_start_secs: u64,
    session_end_secs: u64,
    upload_rate_bps: f64,
    latency_ms: f64,
    reliability: f64,
    bitfield_density: f64,
    peer_quality_label: f64,
}

#[derive(Debug)]
struct BlockEventRow {
    peer_id: u64,
    tick: u64,
    piece_index: u64,
    block_offset: u64,
    request_delivery_latency_ms: f64,
    success: bool,
    bytes: u64,
    uploader_peer_id: u64,
}

#[derive(Debug)]
struct SeederEventRow {
    seeder_id: u64,
    observed_duration_secs: u64,
    censored: bool,
    upload_trend_bps_per_sec: f64,
    completion_rate: f64,
    served_blocks: u64,
    departure_observed: bool,
}

#[derive(Debug)]
struct PieceSnapshotRow {
    tick: u64,
    piece_index: u64,
    availability_count: u64,
    rarity_rank: u64,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();
    let config = SimConfig {
        out: args.out,
        seed: args.seed,
        peers: args.peers,
        pieces: args.pieces,
        duration_secs: args.duration_secs,
        tick_secs: args.tick_secs,
    };

    validate_config(&config)?;
    let output = simulate(&config)?;
    write_outputs(&config.out, &output)?;
    info!("simulation complete");
    Ok(())
}

fn validate_config(config: &SimConfig) -> Result<()> {
    if config.peers == 0 {
        bail!("--peers must be greater than zero");
    }
    if config.pieces == 0 {
        bail!("--pieces must be greater than zero");
    }
    if config.duration_secs == 0 {
        bail!("--duration-secs must be greater than zero");
    }
    if config.tick_secs == 0 {
        bail!("--tick-secs must be greater than zero");
    }
    Ok(())
}

fn simulate(config: &SimConfig) -> Result<SimulationOutput> {
    validate_config(config)?;
    fs::create_dir_all(&config.out)
        .with_context(|| format!("failed to create output directory {}", config.out.display()))?;

    let total_ticks = config.duration_secs.div_ceil(config.tick_secs);
    let mut rng = ChaCha8Rng::seed_from_u64(config.seed);
    let mut peers = generate_peers(config, total_ticks, &mut rng)?;
    let mut events = Vec::new();
    let mut snapshots = Vec::new();

    for tick in 0..total_ticks {
        let active_uploaders = active_uploaders(&peers, tick);
        if active_uploaders.is_empty() {
            maybe_snapshot(tick, &peers, config.pieces, &mut snapshots);
            continue;
        }

        for leecher_id in active_leechers(&peers, tick) {
            if peers[leecher_id].pieces.len() == config.pieces {
                continue;
            }
            let Some(piece) = select_piece(
                leecher_id,
                &peers,
                &active_uploaders,
                config.pieces,
                &mut rng,
            ) else {
                continue;
            };
            let Some(uploader_id) =
                select_uploader(piece, leecher_id, &peers, &active_uploaders, &mut rng)
            else {
                continue;
            };

            let success = rng.gen_bool(peers[uploader_id].reliability);
            let latency =
                sample_delivery_latency(&peers[uploader_id], &peers[leecher_id], &mut rng);
            let block_offset = (tick % 16) * BLOCK_BYTES;

            if success {
                peers[leecher_id].pieces.insert(piece);
                peers[uploader_id].uploaded_bytes += BLOCK_BYTES;
                peers[uploader_id].delivered_blocks += 1;
                peers[uploader_id].latency_sum_ms += latency;
                peers[uploader_id].last_tick = Some(tick);
                peers[uploader_id].first_tick.get_or_insert(tick);
            } else {
                peers[uploader_id].failed_blocks += 1;
            }

            events.push(BlockEventRow {
                peer_id: peers[leecher_id].id,
                tick,
                piece_index: piece as u64,
                block_offset,
                request_delivery_latency_ms: latency,
                success,
                bytes: if success { BLOCK_BYTES } else { 0 },
                uploader_peer_id: peers[uploader_id].id,
            });
        }

        maybe_snapshot(tick, &peers, config.pieces, &mut snapshots);
    }

    let peer_sessions = peers
        .iter()
        .map(|peer| PeerSessionRow {
            peer_id: peer.id,
            role: peer.role,
            client_id: peer.client,
            session_start_secs: peer.start_tick * config.tick_secs,
            session_end_secs: peer.end_tick.min(total_ticks) * config.tick_secs,
            upload_rate_bps: peer.upload_rate_bps,
            latency_ms: peer.latency_ms,
            reliability: peer.reliability,
            bitfield_density: peer.bitfield_density(config.pieces),
            peer_quality_label: peer.quality_label(),
        })
        .collect();

    let seeder_events = peers
        .iter()
        .filter(|peer| peer.role == PeerRole::Seeder)
        .map(|peer| {
            let observed_ticks = peer
                .end_tick
                .min(total_ticks)
                .saturating_sub(peer.start_tick);
            let first = peer.first_tick.unwrap_or(peer.start_tick);
            let last = peer.last_tick.unwrap_or(first);
            let active_secs = ((last.saturating_sub(first)).max(1) * config.tick_secs) as f64;
            SeederEventRow {
                seeder_id: peer.id,
                observed_duration_secs: observed_ticks * config.tick_secs,
                censored: peer.end_tick >= total_ticks,
                upload_trend_bps_per_sec: peer.uploaded_bytes as f64 / active_secs,
                completion_rate: peer.delivered_blocks as f64
                    / (peer.delivered_blocks + peer.failed_blocks).max(1) as f64,
                served_blocks: peer.delivered_blocks,
                departure_observed: peer.end_tick < total_ticks,
            }
        })
        .collect();

    Ok(SimulationOutput {
        peer_sessions,
        block_events: events,
        seeder_events,
        piece_snapshots: snapshots,
    })
}

fn generate_peers(config: &SimConfig, total_ticks: u64, rng: &mut ChaCha8Rng) -> Result<Vec<Peer>> {
    let seeders = (config.peers / 20).max(1);
    let log_normal = LogNormal::new(10.0, 1.0)?;
    let churn = Exp::new(1.0 / (total_ticks.max(1) as f64 * 0.65))?;
    let latency = LogNormal::new(4.2, 0.55)?;
    let upload = LogNormal::new(13.0, 0.9)?;

    let mut peers = Vec::with_capacity(config.peers);
    for index in 0..config.peers {
        let role = if index < seeders {
            PeerRole::Seeder
        } else {
            PeerRole::Leecher
        };
        let start_tick = if role == PeerRole::Seeder {
            0
        } else {
            rng.gen_range(0..total_ticks.max(1))
        };
        let sampled_lifetime = churn.sample(rng).ceil() as u64 + 1;
        let end_tick = if role == PeerRole::Seeder {
            (sampled_lifetime * 2).min(total_ticks + sampled_lifetime / 2)
        } else {
            (start_tick + sampled_lifetime).min(total_ticks)
        };
        let client = sample_client(rng);
        let reliability = sample_reliability(client, rng);
        let mut pieces = BTreeSet::new();
        if role == PeerRole::Seeder {
            pieces.extend(0..config.pieces);
        } else {
            let initial_density = rng.gen_range(0.01..0.25);
            for piece in 0..config.pieces {
                if rng.gen_bool(initial_density) {
                    pieces.insert(piece);
                }
            }
        }

        let upload_rate_bps: f64 = upload.sample(rng);
        let latency_ms: f64 = latency.sample(rng);
        let churn_sample: f64 = log_normal.sample(rng);

        peers.push(Peer {
            id: index as u64,
            role,
            client,
            start_tick,
            end_tick: end_tick.max(start_tick + 1),
            upload_rate_bps: upload_rate_bps.min(25_000_000.0),
            latency_ms: latency_ms.clamp(8.0, 2_500.0),
            reliability,
            churn_tendency: (churn_sample / 100_000.0).clamp(0.0, 1.0),
            pieces,
            uploaded_bytes: 0,
            delivered_blocks: 0,
            failed_blocks: 0,
            latency_sum_ms: 0.0,
            first_tick: None,
            last_tick: None,
        });
    }
    Ok(peers)
}

fn sample_client(rng: &mut ChaCha8Rng) -> ClientFamily {
    let choices = [
        ClientFamily::QBittorrent,
        ClientFamily::Transmission,
        ClientFamily::UTorrent,
        ClientFamily::Libtorrent,
        ClientFamily::Unknown,
    ];
    let weights = [35, 22, 12, 21, 10];
    choices[WeightedIndex::new(weights)
        .expect("valid weights")
        .sample(rng)]
}

fn sample_reliability(client: ClientFamily, rng: &mut ChaCha8Rng) -> f64 {
    let base: f64 = match client {
        ClientFamily::QBittorrent => 0.94,
        ClientFamily::Transmission => 0.91,
        ClientFamily::UTorrent => 0.84,
        ClientFamily::Libtorrent => 0.92,
        ClientFamily::Unknown => 0.78,
    };
    (base + rng.gen_range(-0.12..0.06)).clamp(0.35, 0.995)
}

fn active_uploaders(peers: &[Peer], tick: u64) -> Vec<usize> {
    peers
        .iter()
        .enumerate()
        .filter_map(|(index, peer)| {
            (peer.active_at(tick) && !peer.pieces.is_empty()).then_some(index)
        })
        .collect()
}

fn active_leechers(peers: &[Peer], tick: u64) -> Vec<usize> {
    peers
        .iter()
        .enumerate()
        .filter_map(|(index, peer)| {
            (peer.role == PeerRole::Leecher && peer.active_at(tick)).then_some(index)
        })
        .collect()
}

fn select_piece(
    leecher_id: usize,
    peers: &[Peer],
    active_uploaders: &[usize],
    pieces: usize,
    rng: &mut ChaCha8Rng,
) -> Option<usize> {
    let mut availability = piece_availability(peers, active_uploaders, pieces);
    for piece in &peers[leecher_id].pieces {
        availability.remove(piece);
    }
    let mut candidates = availability
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(piece, count)| (*count, *piece));

    match candidates.as_slice() {
        [] => None,
        [(piece, _)] => Some(*piece),
        [(rarest, _), (second, _), ..] => {
            if rng.gen_bool(0.18) {
                Some(*second)
            } else {
                Some(*rarest)
            }
        }
    }
}

fn select_uploader(
    piece: usize,
    leecher_id: usize,
    peers: &[Peer],
    active_uploaders: &[usize],
    rng: &mut ChaCha8Rng,
) -> Option<usize> {
    let candidates = active_uploaders
        .iter()
        .copied()
        .filter(|id| *id != leecher_id && peers[*id].pieces.contains(&piece))
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return None;
    }

    let weights = candidates
        .iter()
        .map(|id| {
            let peer = &peers[*id];
            ((peer.upload_rate_bps / 100_000.0)
                * peer.reliability
                * (1.0 - peer.churn_tendency * 0.2))
                .max(1.0) as u64
        })
        .collect::<Vec<_>>();
    Some(candidates[WeightedIndex::new(weights).ok()?.sample(rng)])
}

fn sample_delivery_latency(uploader: &Peer, leecher: &Peer, rng: &mut ChaCha8Rng) -> f64 {
    let jitter = Uniform::new(0.85, 1.35).sample(rng);
    (uploader.latency_ms * 0.7 + leecher.latency_ms * 0.3) * jitter
}

fn piece_availability(
    peers: &[Peer],
    active_uploaders: &[usize],
    pieces: usize,
) -> HashMap<usize, u64> {
    let mut availability = HashMap::with_capacity(pieces);
    for peer_id in active_uploaders {
        for piece in &peers[*peer_id].pieces {
            *availability.entry(*piece).or_insert(0) += 1;
        }
    }
    availability
}

fn maybe_snapshot(tick: u64, peers: &[Peer], pieces: usize, snapshots: &mut Vec<PieceSnapshotRow>) {
    if !tick.is_multiple_of(SNAPSHOT_INTERVAL_TICKS) {
        return;
    }
    let active = active_uploaders(peers, tick);
    let mut availability = (0..pieces)
        .map(|piece| {
            let count = active
                .iter()
                .filter(|peer_id| peers[**peer_id].pieces.contains(&piece))
                .count() as u64;
            (piece, count)
        })
        .collect::<Vec<_>>();
    availability.sort_by_key(|(piece, count)| (*count, *piece));

    for (rank, (piece, count)) in availability.into_iter().enumerate() {
        snapshots.push(PieceSnapshotRow {
            tick,
            piece_index: piece as u64,
            availability_count: count,
            rarity_rank: rank as u64,
        });
    }
}

fn write_outputs(out: &Path, output: &SimulationOutput) -> Result<()> {
    fs::create_dir_all(out)
        .with_context(|| format!("failed to create output directory {}", out.display()))?;
    write_peer_sessions(&out.join("peer_sessions.parquet"), &output.peer_sessions)?;
    write_block_events(&out.join("block_events.parquet"), &output.block_events)?;
    write_seeder_events(&out.join("seeder_events.parquet"), &output.seeder_events)?;
    write_piece_snapshots(
        &out.join("piece_snapshots.parquet"),
        &output.piece_snapshots,
    )?;
    Ok(())
}

fn writer_props() -> WriterProperties {
    WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .build()
}

fn write_batch(path: &Path, batch: RecordBatch) -> Result<()> {
    let file =
        File::create(path).with_context(|| format!("failed to create {}", path.display()))?;
    let mut writer = ArrowWriter::try_new(file, batch.schema(), Some(writer_props()))?;
    writer.write(&batch)?;
    writer.close()?;
    Ok(())
}

fn write_peer_sessions(path: &Path, rows: &[PeerSessionRow]) -> Result<()> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("peer_id", DataType::UInt64, false),
        Field::new("role", DataType::Utf8, false),
        Field::new("client_id", DataType::Utf8, false),
        Field::new("session_start_secs", DataType::UInt64, false),
        Field::new("session_end_secs", DataType::UInt64, false),
        Field::new("upload_rate_bps", DataType::Float64, false),
        Field::new("latency_ms", DataType::Float64, false),
        Field::new("reliability", DataType::Float64, false),
        Field::new("bitfield_density", DataType::Float64, false),
        Field::new("peer_quality_label", DataType::Float64, false),
    ]));
    write_batch(
        path,
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(UInt64Array::from_iter_values(
                    rows.iter().map(|r| r.peer_id),
                )),
                Arc::new(StringArray::from_iter_values(
                    rows.iter().map(|r| r.role.as_str()),
                )),
                Arc::new(StringArray::from_iter_values(
                    rows.iter().map(|r| r.client_id.as_str()),
                )),
                Arc::new(UInt64Array::from_iter_values(
                    rows.iter().map(|r| r.session_start_secs),
                )),
                Arc::new(UInt64Array::from_iter_values(
                    rows.iter().map(|r| r.session_end_secs),
                )),
                Arc::new(Float64Array::from_iter_values(
                    rows.iter().map(|r| r.upload_rate_bps),
                )),
                Arc::new(Float64Array::from_iter_values(
                    rows.iter().map(|r| r.latency_ms),
                )),
                Arc::new(Float64Array::from_iter_values(
                    rows.iter().map(|r| r.reliability),
                )),
                Arc::new(Float64Array::from_iter_values(
                    rows.iter().map(|r| r.bitfield_density),
                )),
                Arc::new(Float64Array::from_iter_values(
                    rows.iter().map(|r| r.peer_quality_label),
                )),
            ],
        )?,
    )
}

fn write_block_events(path: &Path, rows: &[BlockEventRow]) -> Result<()> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("peer_id", DataType::UInt64, false),
        Field::new("tick", DataType::UInt64, false),
        Field::new("piece_index", DataType::UInt64, false),
        Field::new("block_offset", DataType::UInt64, false),
        Field::new("request_delivery_latency_ms", DataType::Float64, false),
        Field::new("success", DataType::Boolean, false),
        Field::new("bytes", DataType::UInt64, false),
        Field::new("uploader_peer_id", DataType::UInt64, false),
    ]));
    write_batch(
        path,
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(UInt64Array::from_iter_values(
                    rows.iter().map(|r| r.peer_id),
                )),
                Arc::new(UInt64Array::from_iter_values(rows.iter().map(|r| r.tick))),
                Arc::new(UInt64Array::from_iter_values(
                    rows.iter().map(|r| r.piece_index),
                )),
                Arc::new(UInt64Array::from_iter_values(
                    rows.iter().map(|r| r.block_offset),
                )),
                Arc::new(Float64Array::from_iter_values(
                    rows.iter().map(|r| r.request_delivery_latency_ms),
                )),
                Arc::new(BooleanArray::from_iter(
                    rows.iter().map(|r| Some(r.success)),
                )),
                Arc::new(UInt64Array::from_iter_values(rows.iter().map(|r| r.bytes))),
                Arc::new(UInt64Array::from_iter_values(
                    rows.iter().map(|r| r.uploader_peer_id),
                )),
            ],
        )?,
    )
}

fn write_seeder_events(path: &Path, rows: &[SeederEventRow]) -> Result<()> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("seeder_id", DataType::UInt64, false),
        Field::new("observed_duration_secs", DataType::UInt64, false),
        Field::new("censored", DataType::Boolean, false),
        Field::new("upload_trend_bps_per_sec", DataType::Float64, false),
        Field::new("completion_rate", DataType::Float64, false),
        Field::new("served_blocks", DataType::UInt64, false),
        Field::new("departure_observed", DataType::Boolean, false),
    ]));
    write_batch(
        path,
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(UInt64Array::from_iter_values(
                    rows.iter().map(|r| r.seeder_id),
                )),
                Arc::new(UInt64Array::from_iter_values(
                    rows.iter().map(|r| r.observed_duration_secs),
                )),
                Arc::new(BooleanArray::from_iter(
                    rows.iter().map(|r| Some(r.censored)),
                )),
                Arc::new(Float64Array::from_iter_values(
                    rows.iter().map(|r| r.upload_trend_bps_per_sec),
                )),
                Arc::new(Float64Array::from_iter_values(
                    rows.iter().map(|r| r.completion_rate),
                )),
                Arc::new(UInt64Array::from_iter_values(
                    rows.iter().map(|r| r.served_blocks),
                )),
                Arc::new(BooleanArray::from_iter(
                    rows.iter().map(|r| Some(r.departure_observed)),
                )),
            ],
        )?,
    )
}

fn write_piece_snapshots(path: &Path, rows: &[PieceSnapshotRow]) -> Result<()> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("tick", DataType::UInt64, false),
        Field::new("piece_index", DataType::UInt64, false),
        Field::new("availability_count", DataType::UInt64, false),
        Field::new("rarity_rank", DataType::UInt64, false),
    ]));
    write_batch(
        path,
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(UInt64Array::from_iter_values(rows.iter().map(|r| r.tick))),
                Arc::new(UInt64Array::from_iter_values(
                    rows.iter().map(|r| r.piece_index),
                )),
                Arc::new(UInt64Array::from_iter_values(
                    rows.iter().map(|r| r.availability_count),
                )),
                Arc::new(UInt64Array::from_iter_values(
                    rows.iter().map(|r| r.rarity_rank),
                )),
            ],
        )?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use parquet::file::reader::{FileReader, SerializedFileReader};

    fn test_config(out: PathBuf) -> SimConfig {
        SimConfig {
            out,
            seed: 7,
            peers: 32,
            pieces: 24,
            duration_secs: 600,
            tick_secs: 30,
        }
    }

    #[test]
    fn generation_is_deterministic_for_same_seed() {
        let temp = tempfile::tempdir().unwrap();
        let config = test_config(temp.path().join("out"));
        let first = simulate(&config).unwrap();
        let second = simulate(&config).unwrap();

        assert_eq!(first.peer_sessions.len(), second.peer_sessions.len());
        assert_eq!(first.block_events.len(), second.block_events.len());
        assert_eq!(first.seeder_events.len(), second.seeder_events.len());
        assert_eq!(first.piece_snapshots.len(), second.piece_snapshots.len());
        assert_eq!(
            first
                .block_events
                .iter()
                .map(|row| row.piece_index)
                .collect::<Vec<_>>(),
            second
                .block_events
                .iter()
                .map(|row| row.piece_index)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn peer_quality_labels_are_normalized() {
        let temp = tempfile::tempdir().unwrap();
        let config = test_config(temp.path().join("out"));
        let output = simulate(&config).unwrap();

        assert!(output
            .peer_sessions
            .iter()
            .all(|row| (0.0..=1.0).contains(&row.peer_quality_label)));
    }

    #[test]
    fn piece_availability_is_bounded() {
        let temp = tempfile::tempdir().unwrap();
        let config = test_config(temp.path().join("out"));
        let output = simulate(&config).unwrap();

        assert!(output
            .piece_snapshots
            .iter()
            .all(|row| row.availability_count <= config.peers as u64));
    }

    #[test]
    fn rfwpms_sometimes_chooses_second_rarest_candidate() {
        let mut rng = ChaCha8Rng::seed_from_u64(11);
        let mut peers = vec![
            peer_with_pieces(0, PeerRole::Leecher, &[0]),
            peer_with_pieces(1, PeerRole::Seeder, &[0, 1, 2]),
            peer_with_pieces(2, PeerRole::Seeder, &[0, 2]),
        ];
        peers[0].start_tick = 0;
        peers[0].end_tick = 10;
        let active = vec![1, 2];

        let selections = (0..200)
            .filter_map(|_| select_piece(0, &peers, &active, 3, &mut rng))
            .collect::<Vec<_>>();
        assert!(selections.contains(&1));
        assert!(selections.contains(&2));
    }

    #[test]
    fn invalid_cli_values_are_rejected() {
        let config = SimConfig {
            out: PathBuf::from("ignored"),
            seed: 1,
            peers: 0,
            pieces: 1,
            duration_secs: 1,
            tick_secs: 1,
        };

        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn writes_all_parquet_outputs() {
        let temp = tempfile::tempdir().unwrap();
        let config = test_config(temp.path().join("out"));
        let output = simulate(&config).unwrap();
        write_outputs(&config.out, &output).unwrap();

        for file in [
            "peer_sessions.parquet",
            "block_events.parquet",
            "seeder_events.parquet",
            "piece_snapshots.parquet",
        ] {
            let path = config.out.join(file);
            assert!(path.exists(), "{} missing", path.display());
            let reader = SerializedFileReader::new(File::open(&path).unwrap()).unwrap();
            assert!(reader.metadata().file_metadata().num_rows() > 0);
        }
    }

    fn peer_with_pieces(id: u64, role: PeerRole, pieces: &[usize]) -> Peer {
        Peer {
            id,
            role,
            client: ClientFamily::Unknown,
            start_tick: 0,
            end_tick: 100,
            upload_rate_bps: 1_000_000.0,
            latency_ms: 50.0,
            reliability: 1.0,
            churn_tendency: 0.0,
            pieces: pieces.iter().copied().collect(),
            uploaded_bytes: 0,
            delivered_blocks: 0,
            failed_blocks: 0,
            latency_sum_ms: 0.0,
            first_tick: None,
            last_tick: None,
        }
    }
}
