use url::Url;

use crate::TrackerError;

/// One BEP 12 tracker tier.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackerTier {
    urls: Vec<Url>,
}

impl TrackerTier {
    /// Create a non-empty tracker tier.
    ///
    /// # Errors
    ///
    /// Returns [`TrackerError::EmptyTrackerTier`] when `urls` is empty.
    pub fn new(urls: Vec<Url>) -> Result<Self, TrackerError> {
        if urls.is_empty() {
            return Err(TrackerError::EmptyTrackerTier);
        }
        Ok(Self { urls })
    }

    /// Return URLs in current preference order.
    #[must_use]
    pub fn urls(&self) -> &[Url] {
        &self.urls
    }

    fn promote(&mut self, url: &Url) -> bool {
        let Some(index) = self.urls.iter().position(|candidate| candidate == url) else {
            return false;
        };
        self.urls.swap(0, index);
        true
    }
}

/// BEP 12 multitracker tier list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackerTierList {
    tiers: Vec<TrackerTier>,
}

impl TrackerTierList {
    /// Create a non-empty BEP 12 tier list.
    ///
    /// # Errors
    ///
    /// Returns [`TrackerError::EmptyTrackerTierList`] when no tiers are
    /// supplied, or [`TrackerError::EmptyTrackerTier`] when any tier is empty.
    pub fn new(tiers: Vec<Vec<Url>>) -> Result<Self, TrackerError> {
        if tiers.is_empty() {
            return Err(TrackerError::EmptyTrackerTierList);
        }

        let tiers = tiers
            .into_iter()
            .map(TrackerTier::new)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { tiers })
    }

    /// Return URLs for a tier by index.
    #[must_use]
    pub fn tier_urls(&self, index: usize) -> Option<&[Url]> {
        self.tiers.get(index).map(TrackerTier::urls)
    }

    /// Promote a successful tracker to the front of its current tier.
    ///
    /// # Errors
    ///
    /// Returns [`TrackerError::UnknownTrackerUrl`] when `url` is not present in
    /// any tier.
    pub fn promote(&mut self, url: &Url) -> Result<(), TrackerError> {
        for tier in &mut self.tiers {
            if tier.promote(url) {
                return Ok(());
            }
        }
        Err(TrackerError::UnknownTrackerUrl)
    }
}

#[cfg(test)]
mod tests {
    use url::Url;

    use crate::{TrackerError, TrackerTierList};

    fn url(input: &str) -> Url {
        Url::parse(input).unwrap()
    }

    #[test]
    fn tracker_tier_list_rejects_empty_tiers() {
        let err = TrackerTierList::new(Vec::new()).unwrap_err();

        assert!(matches!(err, TrackerError::EmptyTrackerTierList));
    }

    #[test]
    fn tracker_tier_list_rejects_empty_inner_tier() {
        let err = TrackerTierList::new(vec![Vec::new()]).unwrap_err();

        assert!(matches!(err, TrackerError::EmptyTrackerTier));
    }

    #[test]
    fn promote_successful_tracker_moves_only_within_its_tier() {
        let first = url("https://tracker-a.example/announce");
        let second = url("https://tracker-b.example/announce");
        let third = url("https://tracker-c.example/announce");
        let mut tiers = TrackerTierList::new(vec![
            vec![first.clone(), second.clone()],
            vec![third.clone()],
        ])
        .unwrap();

        tiers.promote(&second).unwrap();

        assert_eq!(tiers.tier_urls(0).unwrap(), &[second, first]);
    }

    #[test]
    fn promote_unknown_tracker_returns_error() {
        let mut tiers =
            TrackerTierList::new(vec![vec![url("https://tracker-a.example/announce")]]).unwrap();

        let err = tiers
            .promote(&url("https://tracker-missing.example/announce"))
            .unwrap_err();

        assert!(matches!(err, TrackerError::UnknownTrackerUrl));
    }
}
