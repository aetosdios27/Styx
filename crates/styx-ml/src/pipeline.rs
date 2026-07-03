use crate::{
    ConnectionSample, FeatureExtractor, FeatureNormalizer, MlError, PolicyThresholds,
    ThrottleDecision, ThrottleModel,
};

/// End-to-end detector for one peer session.
#[derive(Clone, Debug)]
pub struct ThrottleDetector<M> {
    extractor: FeatureExtractor,
    normalizer: FeatureNormalizer,
    model: M,
    policy: PolicyThresholds,
}

impl<M: ThrottleModel> ThrottleDetector<M> {
    pub fn new(model: M, normalizer: FeatureNormalizer) -> Self {
        Self {
            extractor: FeatureExtractor::new(),
            normalizer,
            model,
            policy: PolicyThresholds::default(),
        }
    }

    pub fn with_policy(model: M, normalizer: FeatureNormalizer, policy: PolicyThresholds) -> Self {
        Self {
            extractor: FeatureExtractor::new(),
            normalizer,
            model,
            policy,
        }
    }

    pub fn observe(&mut self, sample: ConnectionSample) -> Result<(), MlError> {
        self.extractor.observe(sample)
    }

    pub fn evaluate(&self) -> Result<ThrottleDecision, MlError> {
        let features = self.extractor.extract()?;
        let normalized = self.normalizer.normalize(&features)?;
        let signal = self.model.predict(&normalized)?;

        Ok(self.policy.decide(signal))
    }
}
