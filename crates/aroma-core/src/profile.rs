//! Algorithm profiles (D6): the reference implementation's tuned knobs (default) and the
//! paper's. A profile's name and full parameter set travel into the manifest and results.

use crate::features::FeatureConfig;
use crate::prune::Objective;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProfileName {
    Reference,
    Paper,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Params {
    /// Phase I: keep overlap > min_similarity * known query features.
    pub min_similarity: f64,
    /// Phase I cap.
    pub num_similars: usize,
    /// Rerank: keep Jaccard(F(q), F(pruned)) > min_pruned (paper tau1).
    pub min_pruned: f64,
    /// Cluster containment threshold.
    pub tau3: f64,
    /// Cluster extension threshold.
    pub tau2: f64,
    /// Maximum number of recommendations.
    pub top_n: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Profile {
    pub name: ProfileName,
    pub features: FeatureConfig,
    pub params: Params,
    pub objective: Objective,
}

impl Profile {
    pub const REFERENCE: Profile = Profile {
        name: ProfileName::Reference,
        features: FeatureConfig::DEFAULT,
        params: Params {
            min_similarity: 0.4,
            num_similars: 100,
            min_pruned: 0.65,
            tau3: 0.9,
            tau2: 1.5,
            top_n: 5,
        },
        objective: Objective::Jaccard,
    };

    pub const PAPER: Profile = Profile {
        name: ProfileName::Paper,
        features: FeatureConfig::DEFAULT,
        params: Params {
            min_similarity: 0.0,
            num_similars: 1000,
            min_pruned: 0.65,
            tau3: 0.9,
            tau2: 1.5,
            top_n: 5,
        },
        objective: Objective::IntersectionSize,
    };

    pub fn by_name(name: ProfileName) -> Profile {
        match name {
            ProfileName::Reference => Profile::REFERENCE,
            ProfileName::Paper => Profile::PAPER,
        }
    }

    /// Same profile with the reference's syntactic `#VAR` marking (`--compat`).
    pub fn compat(self) -> Profile {
        Profile {
            features: FeatureConfig::REFERENCE,
            ..self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn by_name_returns_the_named_profile() {
        assert_eq!(Profile::by_name(ProfileName::Reference), Profile::REFERENCE);
        assert_eq!(Profile::by_name(ProfileName::Paper), Profile::PAPER);
    }

    #[test]
    fn reference_profile_values() {
        let p = Profile::REFERENCE;
        assert_eq!(p.params.min_similarity, 0.4);
        assert_eq!(p.params.num_similars, 100);
        assert_eq!(p.params.min_pruned, 0.65);
        assert_eq!(
            (p.params.tau3, p.params.tau2, p.params.top_n),
            (0.9, 1.5, 5)
        );
        assert_eq!(p.features, FeatureConfig::DEFAULT);
        assert_eq!(p.objective, Objective::Jaccard);
        assert_eq!(Profile::PAPER.params.num_similars, 1000);
        assert_eq!(Profile::PAPER.params.min_similarity, 0.0);
        assert_eq!(Profile::PAPER.objective, Objective::IntersectionSize);
    }

    #[test]
    fn compat_swaps_only_the_feature_config() {
        let c = Profile::PAPER.compat();
        assert_eq!(c.features, FeatureConfig::REFERENCE);
        assert_eq!(c.params, Profile::PAPER.params);
        assert_eq!(c.name, ProfileName::Paper);
        assert_eq!(c.objective, Objective::IntersectionSize);
    }

    #[test]
    fn profile_name_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&ProfileName::Reference).unwrap(),
            "\"reference\""
        );
    }
}
