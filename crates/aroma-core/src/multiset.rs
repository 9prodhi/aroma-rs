//! Feature multisets. The reference uses Python `Counter`: `&` is per-feature min,
//! `|` per-feature max, `+` per-feature sum. A bag is a sorted run-length vector, and
//! every binary operation is one private visitor merge over the union of keys
//! (the glaurung pattern, note 12): the visitor sees `(feature, count_a, count_b)` with
//! a zero for the side that lacks the feature.

use std::cmp::Ordering;

pub type FeatureId = u64;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bag(Vec<(FeatureId, u32)>);

/// Walk the sorted union of `a` and `b`, calling `visit(f, count_in_a, count_in_b)`.
fn merge(
    a: &[(FeatureId, u32)],
    b: &[(FeatureId, u32)],
    mut visit: impl FnMut(FeatureId, u32, u32),
) {
    let (mut i, mut j) = (0, 0);
    loop {
        match (a.get(i), b.get(j)) {
            (Some(&(fa, ca)), Some(&(fb, cb))) => match fa.cmp(&fb) {
                Ordering::Less => {
                    visit(fa, ca, 0);
                    i += 1;
                }
                Ordering::Greater => {
                    visit(fb, 0, cb);
                    j += 1;
                }
                Ordering::Equal => {
                    visit(fa, ca, cb);
                    i += 1;
                    j += 1;
                }
            },
            (Some(&(fa, ca)), None) => {
                visit(fa, ca, 0);
                i += 1;
            }
            (None, Some(&(fb, cb))) => {
                visit(fb, 0, cb);
                j += 1;
            }
            (None, None) => break,
        }
    }
}

impl Bag {
    pub fn from_ids(ids: impl IntoIterator<Item = FeatureId>) -> Bag {
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
        self.0.iter().map(|&(_, c)| u64::from(c)).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `(feature, count)` pairs in ascending feature order; every count is > 0.
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

    fn combine(&self, other: &Bag, op: impl Fn(u32, u32) -> u32) -> Bag {
        let mut out = Vec::with_capacity(self.0.len().max(other.0.len()));
        merge(&self.0, &other.0, |f, a, b| {
            let c = op(a, b);
            if c > 0 {
                out.push((f, c));
            }
        });
        Bag(out)
    }

    /// Multiset intersection (Counter `&`, per-feature min).
    pub fn intersect(&self, o: &Bag) -> Bag {
        self.combine(o, u32::min)
    }

    /// Multiset union (Counter `|`, per-feature max).
    pub fn union(&self, o: &Bag) -> Bag {
        self.combine(o, u32::max)
    }

    /// Multiset sum (Counter `+`, the paper's ⊎). Counts saturate at `u32::MAX`.
    pub fn sum(&self, o: &Bag) -> Bag {
        self.combine(o, u32::saturating_add)
    }

    /// |A ∩ B| without allocating: the paper's SimScore.
    pub fn intersection_size(&self, o: &Bag) -> u64 {
        let mut total = 0u64;
        merge(&self.0, &o.0, |_, a, b| total += u64::from(a.min(b)));
        total
    }

    /// |A ∪ B| with per-feature max.
    pub fn union_size(&self, o: &Bag) -> u64 {
        let mut total = 0u64;
        merge(&self.0, &o.0, |_, a, b| total += u64::from(a.max(b)));
        total
    }
}

/// Weighted Jaccard `Σmin / Σmax` (similar.py `distance`, despite its name).
/// `jaccard(∅, ∅) = 0`: an empty query must never look like a perfect match.
pub fn jaccard(a: &Bag, b: &Bag) -> f64 {
    let (mut smin, mut smax) = (0u64, 0u64);
    merge(&a.0, &b.0, |_, x, y| {
        smin += u64::from(x.min(y));
        smax += u64::from(x.max(y));
    });
    if smax == 0 {
        0.0
    } else {
        smin as f64 / smax as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_visits_the_sorted_union_once() {
        let a: [(FeatureId, u32); 2] = [(1, 2), (3, 1)];
        let b: [(FeatureId, u32); 3] = [(2, 5), (3, 4), (9, 1)];
        let mut seen = Vec::new();
        merge(&a, &b, |f, x, y| seen.push((f, x, y)));
        assert_eq!(seen, [(1, 2, 0), (2, 0, 5), (3, 1, 4), (9, 0, 1)]);
        let mut none: Vec<(FeatureId, u32, u32)> = Vec::new();
        merge(&[], &[], |f, x, y| none.push((f, x, y)));
        assert!(none.is_empty());
    }
}
