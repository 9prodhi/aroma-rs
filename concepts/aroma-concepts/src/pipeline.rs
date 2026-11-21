//! End-to-end recommend(): Phase I search -> Phase II prune and rerank -> Phase III
//! cluster and intersect. Hyper-parameters are the reference values (similar.py:80-107).

use crate::cluster::{cluster, intersect};
use crate::features::{featurize, FeatureConfig, Vocab};
use crate::index::InvertedIndex;
use crate::multiset::Bag;
use crate::prune::rerank;
use crate::spt::Spt;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub min_similarity: f64, // Phase I ratio, 0.4 (not in the paper)
    pub num_similars: usize, // Phase I cap, 100 (paper eta1 ~ 1000)
    pub min_pruned: f64,     // rerank cutoff, 0.65 (paper tau1)
    pub tau3: f64,           // cluster containment, 0.9
    pub tau2: f64,           // cluster extension, 1.5
    pub top_n: usize,        // recommendations, 5 (paper K)
}

impl Params {
    pub const REFERENCE: Self = Self {
        min_similarity: 0.4,
        num_similars: 100,
        min_pruned: 0.65,
        tau3: 0.9,
        tau2: 1.5,
        top_n: 5,
    };
}

pub struct Unit {
    pub spt: Spt,
    pub bag: Bag,
}

#[derive(Default)]
pub struct Corpus {
    pub units: Vec<Unit>,
    pub index: InvertedIndex,
    pub vocab: Vocab,
}

pub struct Recommendation {
    pub members: Vec<u32>,
    pub kept: Vec<bool>,
}

/// Result of one query: the reranked similar units (the "similar code" baseline) and the
/// carved recommendations.
pub struct QueryOutput {
    pub similar: Vec<(u32, f64)>,
    pub recommendations: Vec<Recommendation>,
}

impl Corpus {
    pub fn add(&mut self, spt: Spt, cfg: &FeatureConfig) -> u32 {
        let per_leaf = featurize(&spt, cfg, None);
        let bag = self.vocab.unit_bag(&per_leaf);
        let id = self.units.len() as u32;
        self.index.insert(id, &bag);
        self.units.push(Unit { spt, bag });
        id
    }

    pub fn query_bag(&mut self, query: &Spt, cfg: &FeatureConfig) -> Bag {
        self.vocab.unit_bag(&featurize(query, cfg, None))
    }

    pub fn recommend(&mut self, query: &Spt, cfg: &FeatureConfig, p: &Params) -> Vec<Recommendation> {
        self.query(query, cfg, p, &HashSet::new()).recommendations
    }

    /// Full pipeline; units in `exclude` are invisible (held-out evaluation).
    pub fn query(
        &mut self,
        query: &Spt,
        cfg: &FeatureConfig,
        p: &Params,
        exclude: &HashSet<u32>,
    ) -> QueryOutput {
        let q = self.query_bag(query, cfg);
        let hits = self.index.search(&q, p.min_similarity, p.num_similars + exclude.len());
        let units = &self.units;
        let vocab = &mut self.vocab;
        let cands = rerank(
            &q,
            hits.iter()
                .filter(|(u, _)| !exclude.contains(u))
                .take(p.num_similars)
                .map(|&(u, _)| (u, &units[u as usize].spt, units[u as usize].bag.clone())),
            cfg,
            vocab,
            p.min_pruned,
        );
        let similar = cands.iter().map(|c| (c.unit, c.score)).collect();
        let recommendations = cluster(&cands, p.tau3, p.tau2, p.top_n)
            .into_iter()
            .map(|t| Recommendation {
                kept: intersect(&q, &t, &cands, cfg, vocab),
                members: t.iter().map(|&i| cands[i].unit).collect(),
            })
            .collect();
        QueryOutput { similar, recommendations }
    }
}
