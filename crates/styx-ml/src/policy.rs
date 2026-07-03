use crate::{MlError, ThrottleAction, ThrottleDecision, ThrottleSignal};

/// Probability thresholds used to turn model output into advisory actions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PolicyThresholds {
    medium: f32,
    high: f32,
}

impl Default for PolicyThresholds {
    fn default() -> Self {
        Self {
            medium: 0.3,
            high: 0.7,
        }
    }
}

impl PolicyThresholds {
    pub fn new(medium: f32, high: f32) -> Result<Self, MlError> {
        if !medium.is_finite() || !high.is_finite() || medium < 0.0 || high > 1.0 || medium >= high
        {
            return Err(MlError::InvalidThresholds);
        }

        Ok(Self { medium, high })
    }

    pub fn decide(&self, signal: ThrottleSignal) -> ThrottleDecision {
        let actions = if signal.probability < self.medium {
            vec![ThrottleAction::NoAction]
        } else if signal.probability < self.high {
            vec![ThrottleAction::ReduceRequestRate, ThrottleAction::PreferUtp]
        } else {
            vec![
                ThrottleAction::ReduceRequestRate,
                ThrottleAction::PreferUtp,
                ThrottleAction::RotatePeerId,
                ThrottleAction::DropAndReconnect,
            ]
        };

        ThrottleDecision { signal, actions }
    }
}
