use bytes::Bytes;

/// Errors returned by tracker protocol parsing and clients.
#[derive(Debug, thiserror::Error)]
pub enum TrackerError {
    /// Bencode parser failure while reading an HTTP tracker response.
    #[error(transparent)]
    Bencode(#[from] styx_proto::BencodeError),
    /// Tracker returned a BEP 3 failure reason.
    #[error("tracker failure: {reason_text}", reason_text = String::from_utf8_lossy(reason))]
    TrackerFailure {
        /// Raw failure reason bytes.
        reason: Bytes,
    },
    /// Required response field was absent.
    #[error("missing tracker response field `{field}`")]
    MissingField {
        /// Field name.
        field: &'static str,
    },
    /// Response field had the wrong bencode or packet type.
    #[error("tracker response field `{field}` has the wrong type")]
    WrongType {
        /// Field name.
        field: &'static str,
    },
    /// Integer field could not fit the target type or was otherwise invalid.
    #[error("tracker response field `{field}` is outside the valid integer range")]
    InvalidIntegerRange {
        /// Field name.
        field: &'static str,
    },
    /// A raw info-hash field was not exactly 20 bytes.
    #[error("field `{field}` must be exactly 20 bytes")]
    InvalidInfoHashLength {
        /// Field name.
        field: &'static str,
    },
    /// Peer address could not be parsed.
    #[error("invalid tracker peer address in field `{field}`")]
    InvalidPeerAddress {
        /// Field name.
        field: &'static str,
    },
    /// Compact peer list byte count was not divisible by the expected stride.
    #[error("invalid compact peer list length {actual} for {stride}-byte stride")]
    InvalidCompactPeerLength {
        /// Actual byte count.
        actual: usize,
        /// Required stride.
        stride: usize,
    },
    /// Tracker URL could not be parsed or represented.
    #[error("invalid tracker URL")]
    InvalidUrl,
    /// HTTP tracker response exceeded the configured body cap.
    #[error("tracker response body length {actual} exceeds maximum {max}")]
    ResponseTooLarge {
        /// Actual response length.
        actual: usize,
        /// Configured maximum response length.
        max: usize,
    },
    /// Multitracker tier list was empty.
    #[error("tracker tier list must not be empty")]
    EmptyTrackerTierList,
    /// One multitracker tier was empty.
    #[error("tracker tier must not be empty")]
    EmptyTrackerTier,
    /// Requested tracker URL was not present in the tier list.
    #[error("tracker URL is not present in tier list")]
    UnknownTrackerUrl,
    /// UDP packet was shorter than the required fixed prefix.
    #[error(
        "invalid UDP tracker packet for {context}: got {actual} bytes, need at least {minimum}"
    )]
    InvalidUdpPacket {
        /// Packet context.
        context: &'static str,
        /// Actual packet length.
        actual: usize,
        /// Minimum required length.
        minimum: usize,
    },
    /// UDP response action did not match the request.
    #[error("unexpected UDP tracker action: expected {expected}, got {actual}")]
    UnexpectedUdpAction {
        /// Expected action code.
        expected: i32,
        /// Actual action code.
        actual: i32,
    },
    /// UDP response transaction id did not match the request.
    #[error("UDP tracker transaction id mismatch: expected {expected}, got {actual}")]
    TransactionIdMismatch {
        /// Expected transaction id.
        expected: i32,
        /// Actual transaction id.
        actual: i32,
    },
    /// Cached UDP connection id was used after expiry.
    #[error("UDP tracker connection id expired")]
    ConnectionIdExpired,
    /// Underlying IO failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Underlying HTTP client failure.
    #[error(transparent)]
    Http(#[from] reqwest::Error),
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;

    #[test]
    fn tracker_failure_displays_reason() {
        let err = TrackerError::TrackerFailure {
            reason: Bytes::from_static(b"torrent not found"),
        };

        assert_eq!(err.to_string(), "tracker failure: torrent not found");
    }

    #[test]
    fn invalid_compact_peer_length_displays_actual_and_stride() {
        let err = TrackerError::InvalidCompactPeerLength {
            actual: 7,
            stride: 6,
        };

        assert_eq!(
            err.to_string(),
            "invalid compact peer list length 7 for 6-byte stride"
        );
    }

    #[test]
    fn transaction_id_mismatch_displays_expected_and_actual() {
        let err = TrackerError::TransactionIdMismatch {
            expected: 10,
            actual: 11,
        };

        assert_eq!(
            err.to_string(),
            "UDP tracker transaction id mismatch: expected 10, got 11"
        );
    }
}
