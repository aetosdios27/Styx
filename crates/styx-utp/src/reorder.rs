use std::collections::{BTreeMap, BTreeSet};

use bytes::Bytes;

use crate::{SeqNr, UtpError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReorderOutcome {
    pub data: Vec<Bytes>,
    pub duplicate: bool,
}

#[derive(Clone, Debug)]
pub struct ReorderBuffer {
    next: SeqNr,
    buffered: BTreeMap<SeqNr, Bytes>,
    delivered: BTreeSet<SeqNr>,
    buffered_bytes: usize,
    byte_cap: usize,
}

impl ReorderBuffer {
    #[must_use]
    pub fn new(next: SeqNr, byte_cap: usize) -> Self {
        Self {
            next,
            buffered: BTreeMap::new(),
            delivered: BTreeSet::new(),
            buffered_bytes: 0,
            byte_cap,
        }
    }

    pub fn push(&mut self, seq: SeqNr, payload: Bytes) -> Result<ReorderOutcome, UtpError> {
        if self.delivered.contains(&seq) || self.buffered.contains_key(&seq) {
            return Ok(ReorderOutcome {
                data: Vec::new(),
                duplicate: true,
            });
        }

        if seq == self.next {
            self.delivered.insert(seq);
            self.next = self.next.wrapping_add(1);
            let mut data = vec![payload];
            while let Some(payload) = self.buffered.remove(&self.next) {
                self.buffered_bytes = self.buffered_bytes.saturating_sub(payload.len());
                self.delivered.insert(self.next);
                self.next = self.next.wrapping_add(1);
                data.push(payload);
            }
            return Ok(ReorderOutcome {
                data,
                duplicate: false,
            });
        }

        if self.buffered_bytes + payload.len() > self.byte_cap {
            return Err(UtpError::ResourceLimitExceeded {
                resource: "reorder_buffer",
            });
        }
        self.buffered_bytes += payload.len();
        self.buffered.insert(seq, payload);
        Ok(ReorderOutcome {
            data: Vec::new(),
            duplicate: false,
        })
    }

    #[must_use]
    pub const fn next(&self) -> SeqNr {
        self.next
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;

    #[test]
    fn push_emits_in_order_data() {
        let mut buffer = ReorderBuffer::new(SeqNr::new(1), 1024);

        let outcome = buffer
            .push(SeqNr::new(1), Bytes::from_static(b"a"))
            .unwrap();

        assert_eq!(outcome.data, vec![Bytes::from_static(b"a")]);
    }

    #[test]
    fn push_buffers_out_of_order_until_gap_arrives() {
        let mut buffer = ReorderBuffer::new(SeqNr::new(1), 1024);
        let _ = buffer
            .push(SeqNr::new(2), Bytes::from_static(b"b"))
            .unwrap();

        let outcome = buffer
            .push(SeqNr::new(1), Bytes::from_static(b"a"))
            .unwrap();

        assert_eq!(
            outcome.data,
            vec![Bytes::from_static(b"a"), Bytes::from_static(b"b")]
        );
    }

    #[test]
    fn push_acknowledges_duplicate_without_emitting_twice() {
        let mut buffer = ReorderBuffer::new(SeqNr::new(1), 1024);
        let _ = buffer
            .push(SeqNr::new(1), Bytes::from_static(b"a"))
            .unwrap();

        let outcome = buffer
            .push(SeqNr::new(1), Bytes::from_static(b"a"))
            .unwrap();

        assert!(outcome.duplicate);
        assert!(outcome.data.is_empty());
    }

    #[test]
    fn push_rejects_data_beyond_byte_cap() {
        let mut buffer = ReorderBuffer::new(SeqNr::new(1), 1);

        let err = buffer
            .push(SeqNr::new(3), Bytes::from_static(b"too-large"))
            .unwrap_err();

        assert_eq!(
            err,
            UtpError::ResourceLimitExceeded {
                resource: "reorder_buffer"
            }
        );
    }
}
