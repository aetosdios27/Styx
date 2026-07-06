#![allow(dead_code)]

use std::collections::BTreeMap;

use bytes::Bytes;
use sha1::{Digest as Sha1Digest, Sha1};
use sha2::Sha256;
use styx_proto::{encode, BencodeValue};

pub struct LargeTorrentConfig {
    pub piece_size: u32,
    pub file_count: u32,
    pub total_size: u64,
    pub mode: TorrentMode,
}

pub enum TorrentMode {
    V1,
    V2,
    Hybrid,
}

impl LargeTorrentConfig {
    pub fn piece_count(&self) -> u32 {
        self.total_size.div_ceil(self.piece_size as u64) as u32
    }

    pub fn generate_torrent_bytes(&self) -> Vec<u8> {
        let disk_data = self.generate_disk_data();
        let all_data: Vec<u8> = disk_data.values().flat_map(|b| b.iter().copied()).collect();
        let info_dict = self.build_info_dict(&disk_data, &all_data);
        let mut top = BTreeMap::new();
        top.insert(
            b"announce".to_vec(),
            BencodeValue::Bytes(Bytes::from_static(b"http://tracker.styx.test/announce")),
        );
        top.insert(b"info".to_vec(), info_dict);
        encode(&BencodeValue::Dict(top))
    }

    pub fn generate_disk_data(&self) -> BTreeMap<String, Bytes> {
        let file_count = self.file_count.max(1);
        let base_size = self.total_size / file_count as u64;
        let remainder = self.total_size % file_count as u64;

        let mut data = BTreeMap::new();
        for i in 0..file_count {
            let file_size = if (i as u64) < remainder {
                base_size + 1
            } else {
                base_size
            } as usize;
            let mut content = Vec::with_capacity(file_size);
            for j in 0..file_size {
                content.push((j ^ (i as usize * 0x1F)) as u8);
            }
            let name = if file_count == 1 {
                "single.bin".to_string()
            } else {
                format!("file_{i}.bin")
            };
            data.insert(name, Bytes::from(content));
        }
        data
    }

    fn build_info_dict(
        &self,
        disk_data: &BTreeMap<String, Bytes>,
        all_data: &[u8],
    ) -> BencodeValue {
        let piece_size = self.piece_size;
        let num_pieces = self.piece_count();

        match self.mode {
            TorrentMode::V1 => self.build_v1_info(disk_data, all_data, piece_size, num_pieces),
            TorrentMode::V2 => self.build_v2_info(disk_data, all_data, piece_size, num_pieces),
            TorrentMode::Hybrid => {
                self.build_hybrid_info(disk_data, all_data, piece_size, num_pieces)
            }
        }
    }

    fn build_v1_info(
        &self,
        disk_data: &BTreeMap<String, Bytes>,
        all_data: &[u8],
        piece_size: u32,
        num_pieces: u32,
    ) -> BencodeValue {
        let mut info = BTreeMap::new();
        info.insert(
            b"name".to_vec(),
            BencodeValue::Bytes(Bytes::from_static(b"test_torrent")),
        );
        info.insert(
            b"piece length".to_vec(),
            BencodeValue::Integer(piece_size as i64),
        );

        let v1_hashes = compute_v1_piece_hashes(all_data, piece_size as usize, num_pieces);
        info.insert(
            b"pieces".to_vec(),
            BencodeValue::Bytes(Bytes::from(v1_hashes)),
        );

        if disk_data.len() == 1 {
            let (_, content) = disk_data.iter().next().unwrap();
            info.insert(
                b"length".to_vec(),
                BencodeValue::Integer(content.len() as i64),
            );
        } else {
            let mut files = Vec::new();
            for (path, content) in disk_data {
                let mut file_dict = BTreeMap::new();
                file_dict.insert(
                    b"length".to_vec(),
                    BencodeValue::Integer(content.len() as i64),
                );
                file_dict.insert(
                    b"path".to_vec(),
                    BencodeValue::List(vec![BencodeValue::Bytes(Bytes::copy_from_slice(
                        path.as_bytes(),
                    ))]),
                );
                files.push(BencodeValue::Dict(file_dict));
            }
            info.insert(b"files".to_vec(), BencodeValue::List(files));
        }

        BencodeValue::Dict(info)
    }

    fn build_v2_info(
        &self,
        disk_data: &BTreeMap<String, Bytes>,
        all_data: &[u8],
        piece_size: u32,
        _num_pieces: u32,
    ) -> BencodeValue {
        let mut info = BTreeMap::new();
        info.insert(b"meta version".to_vec(), BencodeValue::Integer(2));
        info.insert(
            b"name".to_vec(),
            BencodeValue::Bytes(Bytes::from_static(b"test_torrent")),
        );
        info.insert(
            b"piece length".to_vec(),
            BencodeValue::Integer(piece_size as i64),
        );

        let pieces_root = compute_merkle_root(all_data, piece_size as usize);
        info.insert(
            b"pieces root".to_vec(),
            BencodeValue::Bytes(Bytes::copy_from_slice(&pieces_root)),
        );

        let file_tree = build_file_tree(disk_data, piece_size as usize);
        info.insert(b"file tree".to_vec(), file_tree);

        BencodeValue::Dict(info)
    }

    fn build_hybrid_info(
        &self,
        disk_data: &BTreeMap<String, Bytes>,
        all_data: &[u8],
        piece_size: u32,
        num_pieces: u32,
    ) -> BencodeValue {
        let mut info = BTreeMap::new();
        info.insert(
            b"name".to_vec(),
            BencodeValue::Bytes(Bytes::from_static(b"test_torrent")),
        );
        info.insert(
            b"piece length".to_vec(),
            BencodeValue::Integer(piece_size as i64),
        );

        let v1_hashes = compute_v1_piece_hashes(all_data, piece_size as usize, num_pieces);
        info.insert(
            b"pieces".to_vec(),
            BencodeValue::Bytes(Bytes::from(v1_hashes)),
        );

        if disk_data.len() == 1 {
            let (_, content) = disk_data.iter().next().unwrap();
            info.insert(
                b"length".to_vec(),
                BencodeValue::Integer(content.len() as i64),
            );
        } else {
            let mut files = Vec::new();
            for (path, content) in disk_data {
                let mut file_dict = BTreeMap::new();
                file_dict.insert(
                    b"length".to_vec(),
                    BencodeValue::Integer(content.len() as i64),
                );
                file_dict.insert(
                    b"path".to_vec(),
                    BencodeValue::List(vec![BencodeValue::Bytes(Bytes::copy_from_slice(
                        path.as_bytes(),
                    ))]),
                );
                files.push(BencodeValue::Dict(file_dict));
            }
            info.insert(b"files".to_vec(), BencodeValue::List(files));
        }

        info.insert(b"meta version".to_vec(), BencodeValue::Integer(2));

        let pieces_root = compute_merkle_root(all_data, piece_size as usize);
        info.insert(
            b"pieces root".to_vec(),
            BencodeValue::Bytes(Bytes::copy_from_slice(&pieces_root)),
        );

        let file_tree = build_file_tree(disk_data, piece_size as usize);
        info.insert(b"file tree".to_vec(), file_tree);

        BencodeValue::Dict(info)
    }
}

fn compute_v1_piece_hashes(data: &[u8], piece_size: usize, num_pieces: u32) -> Vec<u8> {
    let mut hashes = Vec::with_capacity(num_pieces as usize * 20);
    for chunk in data.chunks(piece_size) {
        hashes.extend_from_slice(&Sha1::digest(chunk));
    }
    hashes
}

fn compute_merkle_root(data: &[u8], piece_size: usize) -> [u8; 32] {
    let mut level: Vec<[u8; 32]> = data
        .chunks(piece_size)
        .map(|chunk| Sha256::digest(chunk).into())
        .collect();

    if level.is_empty() {
        return Sha256::digest([]).into();
    }

    while level.len() > 1 {
        let mut next = Vec::with_capacity(level.len().div_ceil(2));
        for pair in level.chunks(2) {
            if pair.len() == 2 {
                let mut combined = Vec::with_capacity(64);
                combined.extend_from_slice(&pair[0]);
                combined.extend_from_slice(&pair[1]);
                next.push(Sha256::digest(&combined).into());
            } else {
                next.push(pair[0]);
            }
        }
        level = next;
    }
    level[0]
}

fn build_file_tree(disk_data: &BTreeMap<String, Bytes>, piece_size: usize) -> BencodeValue {
    let mut tree = BTreeMap::new();
    for (path, content) in disk_data {
        let pieces_root = compute_merkle_root(content, piece_size);
        let mut entry = BTreeMap::new();
        entry.insert(
            b"length".to_vec(),
            BencodeValue::Integer(content.len() as i64),
        );
        entry.insert(
            b"pieces root".to_vec(),
            BencodeValue::Bytes(Bytes::copy_from_slice(&pieces_root)),
        );
        let mut file_leaf = BTreeMap::new();
        file_leaf.insert(b"".to_vec(), BencodeValue::Dict(entry));
        tree.insert(path.as_bytes().to_vec(), BencodeValue::Dict(file_leaf));
    }
    BencodeValue::Dict(tree)
}
