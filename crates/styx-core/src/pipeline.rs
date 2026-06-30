use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use crate::{BlockRequest, CoreError, PeerKey};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InFlightRequest {
    pub peer: PeerKey,
    pub request: BlockRequest,
    pub requested_at: Instant,
    pub last_activity: Instant,
}

#[derive(Clone, Debug)]
pub struct RequestPipeline {
    peer: PeerKey,
    capacity: usize,
    requests: BTreeMap<BlockRequest, InFlightRequest>,
}

impl RequestPipeline {
    pub fn new(peer: PeerKey, capacity: usize) -> Result<Self, CoreError> {
        if capacity == 0 {
            return Err(CoreError::InvalidConfig {
                field: "request_pipeline_depth",
            });
        }
        Ok(Self {
            peer,
            capacity,
            requests: BTreeMap::new(),
        })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.requests.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }

    #[must_use]
    pub fn is_full(&self) -> bool {
        self.requests.len() >= self.capacity
    }

    #[must_use]
    pub fn contains(&self, request: BlockRequest) -> bool {
        self.requests.contains_key(&request)
    }

    pub fn add(&mut self, request: BlockRequest, now: Instant) -> Result<(), CoreError> {
        if self.requests.contains_key(&request) {
            return Err(CoreError::DuplicateRequest { request });
        }
        if self.is_full() {
            return Err(CoreError::PipelineFull { peer: self.peer });
        }
        self.requests.insert(
            request,
            InFlightRequest {
                peer: self.peer,
                request,
                requested_at: now,
                last_activity: now,
            },
        );
        Ok(())
    }

    pub fn complete(&mut self, request: BlockRequest) -> Result<InFlightRequest, CoreError> {
        self.requests
            .remove(&request)
            .ok_or(CoreError::RequestNotInFlight { request })
    }

    pub fn cancel(&mut self, request: BlockRequest) -> Result<InFlightRequest, CoreError> {
        self.complete(request)
    }

    #[must_use]
    pub fn stalled(&self, now: Instant, timeout: Duration) -> Vec<InFlightRequest> {
        self.requests
            .values()
            .filter(|request| now.duration_since(request.last_activity) >= timeout)
            .copied()
            .collect()
    }

    pub fn requests(&self) -> impl Iterator<Item = InFlightRequest> + '_ {
        self.requests.values().copied()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use styx_disk::{BlockLength, BlockOffset, PieceIndex};

    use super::*;

    fn request(index: u32) -> BlockRequest {
        BlockRequest::new(
            PieceIndex::new(index),
            BlockOffset::new(0),
            BlockLength::new(16).unwrap(),
        )
    }

    #[test]
    fn add_accepts_requests_until_capacity() {
        let now = Instant::now();
        let mut pipeline = RequestPipeline::new(PeerKey::new(1), 5).unwrap();

        for index in 0..5 {
            pipeline.add(request(index), now).unwrap();
        }

        assert_eq!(pipeline.len(), 5);
    }

    #[test]
    fn add_rejects_request_past_capacity() {
        let now = Instant::now();
        let mut pipeline = RequestPipeline::new(PeerKey::new(1), 1).unwrap();
        pipeline.add(request(0), now).unwrap();

        let err = pipeline.add(request(1), now).unwrap_err();

        assert_eq!(
            err,
            CoreError::PipelineFull {
                peer: PeerKey::new(1)
            }
        );
    }

    #[test]
    fn add_rejects_duplicate_request() {
        let now = Instant::now();
        let mut pipeline = RequestPipeline::new(PeerKey::new(1), 5).unwrap();
        let request = request(0);
        pipeline.add(request, now).unwrap();

        let err = pipeline.add(request, now).unwrap_err();

        assert_eq!(err, CoreError::DuplicateRequest { request });
    }

    #[test]
    fn complete_frees_capacity() {
        let now = Instant::now();
        let mut pipeline = RequestPipeline::new(PeerKey::new(1), 1).unwrap();
        let first = request(0);
        pipeline.add(first, now).unwrap();
        pipeline.complete(first).unwrap();
        pipeline.add(request(1), now).unwrap();

        assert_eq!(pipeline.len(), 1);
    }

    #[test]
    fn stalled_returns_requests_older_than_timeout() {
        let now = Instant::now();
        let mut pipeline = RequestPipeline::new(PeerKey::new(1), 5).unwrap();
        let request = request(0);
        pipeline.add(request, now).unwrap();

        let stalled = pipeline.stalled(now + Duration::from_secs(31), Duration::from_secs(30));

        assert_eq!(stalled[0].request, request);
    }

    #[test]
    fn complete_rejects_unknown_request() {
        let mut pipeline = RequestPipeline::new(PeerKey::new(1), 5).unwrap();
        let request = request(0);

        let err = pipeline.complete(request).unwrap_err();

        assert_eq!(err, CoreError::RequestNotInFlight { request });
    }
}
