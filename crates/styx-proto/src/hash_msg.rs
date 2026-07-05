use crate::info_hash_v2::InfoHashV2;

pub const HASH_REQUEST_ID: u8 = 21;
pub const HASHES_ID: u8 = 22;
pub const HASH_REJECT_ID: u8 = 23;

/// BEP 52 hash request message (ID 21)
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HashRequest {
    pub pieces_root: InfoHashV2,
    pub base_layer: u32,
    pub index: u32,
    pub length: u32,
    pub proof_layers: u32,
}

impl HashRequest {
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(1 + 32 + 4 * 4);
        buf.push(HASH_REQUEST_ID);
        buf.extend_from_slice(self.pieces_root.as_bytes());
        buf.extend_from_slice(&self.base_layer.to_be_bytes());
        buf.extend_from_slice(&self.index.to_be_bytes());
        buf.extend_from_slice(&self.length.to_be_bytes());
        buf.extend_from_slice(&self.proof_layers.to_be_bytes());
        buf
    }

    pub fn decode(data: &[u8]) -> Result<Self, HashMsgError> {
        if data.len() < 1 + 32 + 16 {
            return Err(HashMsgError::Truncated);
        }
        if data[0] != HASH_REQUEST_ID {
            return Err(HashMsgError::WrongMessageId(data[0]));
        }
        let mut root_bytes = [0u8; 32];
        root_bytes.copy_from_slice(&data[1..33]);
        Ok(Self {
            pieces_root: InfoHashV2::from(&root_bytes),
            base_layer: u32::from_be_bytes(data[33..37].try_into().unwrap()),
            index: u32::from_be_bytes(data[37..41].try_into().unwrap()),
            length: u32::from_be_bytes(data[41..45].try_into().unwrap()),
            proof_layers: u32::from_be_bytes(data[45..49].try_into().unwrap()),
        })
    }
}

/// BEP 52 hashes response message (ID 22)
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HashesMessage {
    pub pieces_root: InfoHashV2,
    pub base_layer: u32,
    pub index: u32,
    pub length: u32,
    pub proof_layers: u32,
    pub hashes: Vec<[u8; 32]>,
}

impl HashesMessage {
    pub fn encode(&self) -> Vec<u8> {
        let hash_count = self.hashes.len() as u32;
        let mut buf = Vec::with_capacity(1 + 32 + 4 * 5 + hash_count as usize * 32);
        buf.push(HASHES_ID);
        buf.extend_from_slice(self.pieces_root.as_bytes());
        buf.extend_from_slice(&self.base_layer.to_be_bytes());
        buf.extend_from_slice(&self.index.to_be_bytes());
        buf.extend_from_slice(&self.length.to_be_bytes());
        buf.extend_from_slice(&self.proof_layers.to_be_bytes());
        for h in &self.hashes {
            buf.extend_from_slice(h);
        }
        buf
    }

    pub fn decode(data: &[u8]) -> Result<Self, HashMsgError> {
        if data.len() < 1 + 32 + 20 {
            return Err(HashMsgError::Truncated);
        }
        if data[0] != HASHES_ID {
            return Err(HashMsgError::WrongMessageId(data[0]));
        }
        let mut root_bytes = [0u8; 32];
        root_bytes.copy_from_slice(&data[1..33]);
        let base_layer = u32::from_be_bytes(data[33..37].try_into().unwrap());
        let index = u32::from_be_bytes(data[37..41].try_into().unwrap());
        let length = u32::from_be_bytes(data[41..45].try_into().unwrap());
        let proof_layers = u32::from_be_bytes(data[45..49].try_into().unwrap());

        let hash_data = &data[49..];
        if hash_data.len() % 32 != 0 {
            return Err(HashMsgError::InvalidHashData);
        }
        let hashes: Vec<[u8; 32]> = hash_data
            .chunks_exact(32)
            .map(|c| {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(c);
                arr
            })
            .collect();

        Ok(Self {
            pieces_root: InfoHashV2::from(&root_bytes),
            base_layer,
            index,
            length,
            proof_layers,
            hashes,
        })
    }
}

/// BEP 52 hash reject message (ID 23) — same payload as hash request
pub type HashReject = HashRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HashMsgError {
    Truncated,
    WrongMessageId(u8),
    InvalidHashData,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_hash_request() {
        let request = HashRequest {
            pieces_root: InfoHashV2::from(&[0xAB; 32]),
            base_layer: 0,
            index: 0,
            length: 4,
            proof_layers: 2,
        };

        let bytes = request.encode();
        let decoded = HashRequest::decode(&bytes).unwrap();
        assert_eq!(request.pieces_root, decoded.pieces_root);
        assert_eq!(request.base_layer, decoded.base_layer);
        assert_eq!(request.index, decoded.index);
        assert_eq!(request.length, decoded.length);
        assert_eq!(request.proof_layers, decoded.proof_layers);
    }

    #[test]
    fn encode_decode_hashes_message() {
        let hashes_msg = HashesMessage {
            pieces_root: InfoHashV2::from(&[0xAB; 32]),
            base_layer: 0,
            index: 0,
            length: 2,
            proof_layers: 1,
            hashes: vec![[0x01; 32], [0x02; 32], [0x03; 32]],
        };

        let bytes = hashes_msg.encode();
        let decoded = HashesMessage::decode(&bytes).unwrap();
        assert_eq!(hashes_msg.hashes.len(), decoded.hashes.len());
        assert_eq!(hashes_msg.pieces_root, decoded.pieces_root);
        assert_eq!(hashes_msg.base_layer, decoded.base_layer);
        assert_eq!(hashes_msg.index, decoded.index);
        assert_eq!(hashes_msg.length, decoded.length);
        assert_eq!(hashes_msg.proof_layers, decoded.proof_layers);
        for (a, b) in hashes_msg.hashes.iter().zip(decoded.hashes.iter()) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn hash_reject_is_hash_request_type() {
        let reject = HashReject {
            pieces_root: InfoHashV2::from(&[0xCD; 32]),
            base_layer: 1,
            index: 2,
            length: 4,
            proof_layers: 0,
        };
        let bytes = reject.encode();
        let decoded = HashRequest::decode(&bytes).unwrap();
        assert_eq!(decoded.pieces_root, reject.pieces_root);
        assert_eq!(decoded.base_layer, reject.base_layer);
    }
}
