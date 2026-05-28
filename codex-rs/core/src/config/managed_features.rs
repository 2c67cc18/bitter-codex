use std::collections::BTreeMap;

use codex_config::Constrained;
use codex_config::ConstraintResult;

use codex_features::Feature;
use codex_features::Features;

#[derive(Debug, Clone, PartialEq)]
pub struct ManagedFeatures {
    value: Constrained<Features>,
    pinned_features: BTreeMap<Feature, bool>,
}

impl Default for ManagedFeatures {
    fn default() -> Self {
        Self {
            value: Constrained::allow_any(Features::default()),
            pinned_features: BTreeMap::new(),
        }
    }
}

impl ManagedFeatures {
    pub(crate) fn from_configured_with_warnings(
        configured_features: Features,
        startup_warnings: &mut Vec<String>,
    ) -> std::io::Result<Self> {
        let _ = startup_warnings;
        let pinned_features = BTreeMap::new();

        let normalized_features = normalize_candidate(configured_features, &pinned_features);
        Ok(Self {
            value: Constrained::allow_any(normalized_features),
            pinned_features,
        })
    }

    pub fn get(&self) -> &Features {
        self.value.get()
    }

    fn normalize_and_validate(&self, candidate: Features) -> ConstraintResult<Features> {
        let normalized = normalize_candidate(candidate, &self.pinned_features);
        self.value.can_set(&normalized)?;
        Ok(normalized)
    }

    pub fn can_set(&self, candidate: &Features) -> ConstraintResult<()> {
        self.normalize_and_validate(candidate.clone()).map(|_| ())
    }

    pub fn set(&mut self, candidate: Features) -> ConstraintResult<()> {
        let normalized = self.normalize_and_validate(candidate)?;
        self.value.set(normalized)
    }

    pub fn set_enabled(&mut self, feature: Feature, enabled: bool) -> ConstraintResult<()> {
        let mut next = self.get().clone();
        next.set_enabled(feature, enabled);
        self.set(next)
    }

    pub fn enable(&mut self, feature: Feature) -> ConstraintResult<()> {
        self.set_enabled(feature, true)
    }

    pub fn disable(&mut self, feature: Feature) -> ConstraintResult<()> {
        self.set_enabled(feature, false)
    }
}

#[cfg(test)]
impl From<Features> for ManagedFeatures {
    fn from(features: Features) -> Self {
        Self {
            value: Constrained::allow_any(features),
            pinned_features: BTreeMap::new(),
        }
    }
}

impl std::ops::Deref for ManagedFeatures {
    type Target = Features;

    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

fn normalize_candidate(
    mut candidate: Features,
    pinned_features: &BTreeMap<Feature, bool>,
) -> Features {
    for (feature, enabled) in pinned_features {
        candidate.set_enabled(*feature, *enabled);
    }
    candidate.normalize_dependencies();
    candidate
}
