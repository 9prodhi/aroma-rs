//! Feature multisets. The reference uses Python `Counter`: `&` is per-feature min,
//! `|` per-feature max, `+` per-feature sum. Here a bag is a sorted run-length vector so
//! every binary operation is a linear two-pointer merge.

use std::cmp::Ordering;

pub type FeatureId = u32;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bag(Vec<(FeatureId, u32)>);

impl Bag {
    pub fn from_ids(ids: impl IntoIterator<Item = FeatureId>) -> Self {
        let mut v: Vec<FeatureId> = ids.into_iter().collect();
        v.sort_unstable();
        let mut out: Vec<(FeatureId, u32)> = Vec::new();
        for id in v {
            match out.last_mut() {
                Some((last, c)) if *last == id => *c += 1,
                _ => out.push((id, 1)),
            }
        }
        Bag(out)
    }

    /// Total number of occurrences, |F|.
    pub fn len(&self) -> u64 {
        self.0.iter().map(|&(_, c)| c as u64).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (FeatureId, u32)> + '_ {
        self.0.iter().copied()
    }

    /// S(F): the underlying set, used by Phase I.
    pub fn distinct(&self) -> impl Iterator<Item = FeatureId> + '_ {
        self.0.iter().map(|&(f, _)| f)
    }

    pub fn get(&self, f: FeatureId) -> u32 {
        self.0
            .binary_search_by_key(&f, |&(id, _)| id)
            .map_or(0, |i| self.0[i].1)
    }

    fn merge(&self, other: &Bag, mut combine: impl FnMut(u32, u32) -> u32) -> Bag {
        let (a, b) = (&self.0, &other.0);
        let (mut i, mut j) = (0, 0);
        let mut out = Vec::with_capacity(a.len().max(b.len()));
        while i < a.len() || j < b.len() {
            let (f, x, y) = match (a.get(i), b.get(j)) {
                (Some(&(fa, ca)), Some(&(fb, cb))) => match fa.cmp(&fb) {
                    Ordering::Less => { i += 1; (fa, ca, 0) }
                    Ordering::Greater => { j += 1; (fb, 0, cb) }
                    Ordering::Equal => { i += 1; j += 1; (fa, ca, cb) }
                },
                (Some(&(fa, ca)), None) => { i += 1; (fa, ca, 0) }
                (None, Some(&(fb, cb))) => { j += 1; (fb, 0, cb) }
                (None, None) => unreachable!(),
            };
            let c = combine(x, y);
            if c > 0 {
                out.push((f, c));
            }
        }
        Bag(out)
    }

    /// Multiset intersection (Counter `&`).
    pub fn intersect(&self, other: &Bag) -> Bag {
        self.merge(other, u32::min)
    }

    /// Multiset sum (Counter `+`, the paper's ⊎).
    pub fn sum(&self, other: &Bag) -> Bag {
        self.merge(other, |x, y| x + y)
    }

    /// |A ∩ B| without allocating: the paper's SimScore.
    pub fn intersection_size(&self, other: &Bag) -> u64 {
        let mut total = 0u64;
        let (mut i, mut j) = (0, 0);
        while let (Some(&(fa, ca)), Some(&(fb, cb))) = (self.0.get(i), other.0.get(j)) {
            match fa.cmp(&fb) {
                Ordering::Less => i += 1,
                Ordering::Greater => j += 1,
                Ordering::Equal => {
                    total += ca.min(cb) as u64;
                    i += 1;
                    j += 1;
                }
            }
        }
        total
    }

    /// |A ∪ B| with per-feature max (Counter `|`).
    pub fn union_size(&self, other: &Bag) -> u64 {
        self.len() + other.len() - self.intersection_size(other)
    }
}

/// Weighted Jaccard `Σmin / Σmax` (similar.py `distance`, despite its name).
pub fn jaccard(a: &Bag, b: &Bag) -> f64 {
    let u = a.union_size(b);
    if u == 0 { 0.0 } else { a.intersection_size(b) as f64 / u as f64 }
}
