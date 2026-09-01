//! Shared residual-expression algebra for the two symbolic backends
//! ([`derivative`](crate::derivative) and [`antimirov`](crate::antimirov)).
//!
//! Both backends derive regular expressions symbolically rather than
//! building an explicit automaton up front, and both need the same
//! underlying term representation to do it: a hash-consed `Reg` (so
//! product states can share structure and compare/hash in O(1)), smart
//! constructors that keep it normalized (empty/identity elimination,
//! flattened concatenation, sorted deduplicated alternation), and the
//! handful of structural queries (`nullable`, `first_sets`) every search
//! needs regardless of *how* it steps through the term.
//!
//! What's deliberately **not** here: how a `Reg` is actually stepped.
//! Brzozowski derivatives (`derivative.rs`) compute one residual `Reg` per
//! character; Antimirov partial derivatives (`antimirov.rs`) compute a
//! *set* of residuals (a linear form) per character. That's a genuine
//! algorithmic difference, not incidental duplication, so `Reg::derivative`
//! and `partial_der`/`LinearForm` stay in their respective backends.
//!
//! This module used to be two independent copies, one per backend --
//! "written independently... so a bug in one copy doesn't automatically
//! show up in the other," the same reasoning (and the same failure mode)
//! behind the `alphabet_partition` duplication `charset.rs` now
//! consolidates. In practice the two copies just drifted: `Reg::alt`'s
//! hash-based dedup-then-sort (an O(1)-hashing-enabled improvement over the
//! naive sort-then-dedup, documented where it was first introduced) was
//! ported to only one of the two identical implementations, and
//! `expand_repeat`'s `Vec::with_capacity` sizing hint likewise. Neither
//! drift changed either backend's answers -- both were performance-only --
//! but there's no reason a *future* one has to be as harmless, and no way
//! to be sure of that in advance from two copies whose whole premise is
//! that a change to one doesn't have to be reflected in the other.

use crate::ast::{Expr, ExprKind};
use crate::charset::CharSet;
use std::cmp::Ordering;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

/// Normalized residual expression. Interned via [`Rc`] so product states can
/// share structure and cheaply clone keys for the visited set.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) enum RegKind {
    /// Empty language ∅.
    Null,
    /// Language {ε}.
    Eps,
    /// Single-character language drawn from a (possibly multi-interval) set.
    Atom(CharSet),
    /// Ordered concatenation; never empty, never nested Concat at the top.
    Concat(Vec<Reg>),
    /// Sorted, duplicate-free alternation; never empty, never nested Alt.
    Alt(Vec<Reg>),
    /// Kleene star.
    Star(Reg),
}

/// Computes a `RegKind`'s hash once, at construction time (see `Reg::wrap`).
///
/// This is the basis for hash-consing `Reg`: because `RegKind`'s derived
/// `Hash` impl hashes its `Vec<Reg>`/`Reg` fields through *their* `Hash`
/// impl, and `Reg::hash` (below) is overridden to just return this cached
/// value in O(1), computing a *new* parent node's hash only means
/// combining its immediate children's already-cached hashes -- not
/// re-walking their entire subtrees. Without this, hashing the same large
/// shared subterm repeatedly (e.g. as a product-search cache key, or while
/// deduplicating a wide alternation) costs O(size) *every single time*,
/// hit or miss.
fn compute_hash(kind: &RegKind) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    kind.hash(&mut hasher);
    hasher.finish()
}

#[derive(Clone, Debug)]
pub(crate) struct Reg(Rc<(RegKind, u64)>);

impl Hash for Reg {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // O(1): just the cached hash, never a tree walk. See `compute_hash`.
        self.0.as_ref().1.hash(state);
    }
}

impl PartialEq for Reg {
    fn eq(&self, other: &Self) -> bool {
        // Fast rejection on the cached hash before ever touching the
        // (potentially large) `RegKind` comparison. The `RegKind` compare
        // only runs when hashes already match -- true equality, or an
        // astronomically rare 64-bit collision.
        self.0.as_ref().1 == other.0.as_ref().1 && self.0.as_ref().0 == other.0.as_ref().0
    }
}
impl Eq for Reg {}

impl Reg {
    pub(crate) fn kind(&self) -> &RegKind {
        &self.0.as_ref().0
    }

    /// Build a `Reg` from a fresh `RegKind`, computing and caching its
    /// hash once. Every smart constructor below goes through this instead
    /// of `Self(Rc::new(...))` directly.
    fn wrap(kind: RegKind) -> Self {
        let h = compute_hash(&kind);
        Self(Rc::new((kind, h)))
    }

    pub(crate) fn null() -> Self {
        Self::wrap(RegKind::Null)
    }

    pub(crate) fn eps() -> Self {
        Self::wrap(RegKind::Eps)
    }

    pub(crate) fn atom(set: CharSet) -> Self {
        if set.is_empty() {
            Self::null()
        } else {
            Self::wrap(RegKind::Atom(set))
        }
    }

    pub(crate) fn star(inner: Reg) -> Self {
        match inner.kind() {
            RegKind::Null | RegKind::Eps => Self::eps(),
            RegKind::Star(_) => inner,
            _ => Self::wrap(RegKind::Star(inner)),
        }
    }

    pub(crate) fn concat(parts: Vec<Reg>) -> Self {
        let mut flat = Vec::new();
        for part in parts {
            match part.kind() {
                RegKind::Null => return Self::null(),
                RegKind::Eps => {}
                RegKind::Concat(inner) => flat.extend(inner.iter().cloned()),
                _ => flat.push(part),
            }
        }
        match flat.len() {
            0 => Self::eps(),
            1 => flat.pop().unwrap(),
            _ => Self::wrap(RegKind::Concat(flat)),
        }
    }

    pub(crate) fn alt(parts: Vec<Reg>) -> Self {
        let mut flat = Vec::new();
        for part in parts {
            match part.kind() {
                RegKind::Null => {}
                RegKind::Alt(inner) => flat.extend(inner.iter().cloned()),
                _ => flat.push(part),
            }
        }
        // Hash-based dedup, not sort-then-dedup: cheap now that hashing a
        // `Reg` is O(1) (see `compute_hash`), and avoids paying for a full
        // `O(k log k)` comparison-based sort over entries that might
        // largely be duplicates before ever throwing most of them away.
        // The final canonical order still needs `reg_ord`, so the
        // (now much smaller, deduplicated) survivors get sorted after.
        let mut seen: std::collections::HashSet<Reg> =
            std::collections::HashSet::with_capacity(flat.len());
        for r in flat {
            seen.insert(r);
        }
        let mut flat: Vec<Reg> = seen.into_iter().collect();
        flat.sort_by(reg_ord);
        match flat.len() {
            0 => Self::null(),
            1 => flat.pop().unwrap(),
            _ => Self::wrap(RegKind::Alt(flat)),
        }
    }

    /// Concatenate `self` on the left of `tail`, with normalization.
    pub(crate) fn then(self, tail: Reg) -> Reg {
        Reg::concat(vec![self, tail])
    }

    pub(crate) fn nullable(&self) -> bool {
        match self.kind() {
            RegKind::Null | RegKind::Atom(_) => false,
            RegKind::Eps | RegKind::Star(_) => true,
            RegKind::Concat(parts) => parts.iter().all(Reg::nullable),
            RegKind::Alt(parts) => parts.iter().any(Reg::nullable),
        }
    }

    /// Collect outermost character sets that can start a non-empty word.
    pub(crate) fn first_sets(&self, out: &mut Vec<CharSet>) {
        match self.kind() {
            RegKind::Null | RegKind::Eps => {}
            RegKind::Atom(set) => out.push(set.clone()),
            RegKind::Concat(parts) => {
                for part in parts {
                    part.first_sets(out);
                    if !part.nullable() {
                        break;
                    }
                }
            }
            RegKind::Alt(parts) => {
                for part in parts {
                    part.first_sets(out);
                }
            }
            RegKind::Star(inner) => inner.first_sets(out),
        }
    }
}

/// Deterministic total order for alternation sorting / dedup.
pub(crate) fn reg_ord(a: &Reg, b: &Reg) -> Ordering {
    fn rank(k: &RegKind) -> u8 {
        match k {
            RegKind::Null => 0,
            RegKind::Eps => 1,
            RegKind::Atom(_) => 2,
            RegKind::Concat(_) => 3,
            RegKind::Alt(_) => 4,
            RegKind::Star(_) => 5,
        }
    }
    let (ka, kb) = (a.kind(), b.kind());
    match rank(ka).cmp(&rank(kb)) {
        Ordering::Equal => {}
        other => return other,
    }
    match (ka, kb) {
        (RegKind::Null, RegKind::Null) | (RegKind::Eps, RegKind::Eps) => Ordering::Equal,
        (RegKind::Atom(sa), RegKind::Atom(sb)) => {
            let ia = sa.intervals();
            let ib = sb.intervals();
            // Length first: sets with a different number of intervals can
            // never be equal, so this decides most unequal pairs in O(1)
            // instead of walking min(len) intervals first. Same rationale
            // as the Concat/Alt cases below.
            match ia.len().cmp(&ib.len()) {
                Ordering::Equal => {}
                other => return other,
            }
            for (x, y) in ia.iter().zip(ib.iter()) {
                match (x.start, x.end).cmp(&(y.start, y.end)) {
                    Ordering::Equal => {}
                    other => return other,
                }
            }
            Ordering::Equal
        }
        (RegKind::Concat(pa), RegKind::Concat(pb)) | (RegKind::Alt(pa), RegKind::Alt(pb)) => {
            // Length first, *then* elementwise -- not the reverse. Two
            // lists of different length can never be equal, so checking
            // length up front lets most comparisons between
            // differently-sized terms finish in O(1) instead of walking
            // min(len) shared elements only to fall back to the same
            // length check anyway. Matters far more than it looks: for a
            // pattern like 500 concatenated `a?`s (no wrapping `*`), each
            // derivative step's residual is a `Concat`/`Alt` chain that's
            // literally a suffix of the one before it, differing *only* in
            // length -- comparing elementwise-first walks every shared
            // (trivially equal) element before ever reaching the part that
            // actually distinguishes them.
            match pa.len().cmp(&pb.len()) {
                Ordering::Equal => {}
                other => return other,
            }
            for (x, y) in pa.iter().zip(pb.iter()) {
                match reg_ord(x, y) {
                    Ordering::Equal => {}
                    other => return other,
                }
            }
            Ordering::Equal
        }
        (RegKind::Star(ia), RegKind::Star(ib)) => reg_ord(ia, ib),
        _ => Ordering::Equal,
    }
}

pub(crate) fn from_expr(expr: &Expr) -> Reg {
    match &expr.kind {
        ExprKind::Empty | ExprKind::AnchorStart | ExprKind::AnchorEnd => Reg::eps(),
        ExprKind::Literal(ch) => Reg::atom(CharSet::singleton(*ch)),
        ExprKind::CharSet(set) => Reg::atom(set.clone()),
        ExprKind::Concat(parts) => {
            if parts.is_empty() {
                Reg::eps()
            } else {
                Reg::concat(parts.iter().map(from_expr).collect())
            }
        }
        ExprKind::Alt(branches) => {
            if branches.is_empty() {
                Reg::null()
            } else {
                Reg::alt(branches.iter().map(from_expr).collect())
            }
        }
        ExprKind::Repeat { expr, min, max } => expand_repeat(from_expr(expr), *min, *max),
    }
}

pub(crate) fn expand_repeat(inner: Reg, min: usize, max: Option<usize>) -> Reg {
    let mut parts = Vec::with_capacity(min.saturating_add(1));
    for _ in 0..min {
        parts.push(inner.clone());
    }
    match max {
        None => {
            parts.push(Reg::star(inner));
            Reg::concat(parts)
        }
        Some(maximum) => {
            // After the required `min` copies, each remaining slot up to
            // `maximum` is optional: (ε | inner) chained `maximum - min` times.
            for _ in min..maximum {
                parts.push(Reg::alt(vec![Reg::eps(), inner.clone()]));
            }
            Reg::concat(parts)
        }
    }
}

/// Whether a product-search pair of dead-flags means the pair can never
/// contribute to `query`'s answer and is safe to prune -- shared decision
/// core for `derivative::is_dead_end`/`antimirov::is_dead_end`, which each
/// supply their own notion of "dead" for their own residual representation
/// (`RegKind::Null` for a single `Reg`; an empty `LinearForm` for a set of
/// them) and defer the actual query-specific logic here.
///
/// The condition is deliberately query-specific and asymmetric -- getting
/// it wrong in the "more aggressive" direction produces an unsound early
/// `Exhausted`, not just a slower search:
///
/// - [`crate::analysis::Query::Overlap`] needs `left_accepts &&
///   right_accepts` simultaneously; once *either* side is dead that
///   conjunction can never hold again, so pruning on either side alone is
///   sound.
/// - [`crate::analysis::Query::Includes`] needs `left_accepts &&
///   !right_accepts`; once `left` is dead, `left_accepts` is false forever,
///   so the conjunction can never hold regardless of what `right` does.
///   The mirror case is *not* safe to prune: `right` going dead makes
///   `!right_accepts` permanently true, which only helps satisfy the
///   condition the next time `left` accepts, so `left`'s side must keep
///   being explored.
/// - [`crate::analysis::Query::Equivalent`] fires on either `left_only` or
///   `right_only`, so a lone dead side still leaves the other side capable
///   of firing the opposite branch later. Pruning is only sound once
///   *both* sides are dead.
pub(crate) fn dead_end_verdict(
    query: crate::analysis::Query,
    left_dead: bool,
    right_dead: bool,
) -> bool {
    use crate::analysis::Query;
    match query {
        Query::Overlap => left_dead || right_dead,
        Query::Includes => left_dead,
        Query::Equivalent => left_dead && right_dead,
        // Only ever called from a binary product search, which only runs
        // for the three queries above; `Query::Empty` takes a unary search
        // path instead and `Query::Match` isn't a product search at all.
        // `false` (never prune) is the conservative, always-safe answer for
        // a query this function isn't actually asked about.
        Query::Empty | Query::Match => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::parser::parse;

    #[test]
    fn alt_flattens_dedups_and_sorts() {
        let a = Reg::atom(CharSet::singleton('a'));
        let b = Reg::atom(CharSet::singleton('b'));
        let nested = Reg::alt(vec![a.clone(), b.clone()]);
        // Nested Alt flattens, exact duplicates collapse, Null vanishes.
        let combined = Reg::alt(vec![nested, a.clone(), Reg::null(), b.clone()]);
        match combined.kind() {
            RegKind::Alt(parts) => assert_eq!(parts.len(), 2, "expected exactly {{a, b}}"),
            other => panic!("expected Alt, got {other:?}"),
        }
    }

    #[test]
    fn concat_eliminates_identity_and_absorbs_null() {
        let a = Reg::atom(CharSet::singleton('a'));
        assert_eq!(Reg::concat(vec![Reg::eps(), a.clone()]), a);
        assert_eq!(Reg::concat(vec![a.clone(), Reg::null()]), Reg::null());
    }

    #[test]
    fn from_expr_and_nullable_agree_with_parser_on_optional() {
        let expr = parse("a?", &Config::default()).unwrap();
        let reg = from_expr(&expr);
        // "a?" as a whole isn't nullable's target here (it's the group,
        // not epsilon) -- check the concrete language-level property that
        // actually matters: the derivative algebra must agree that the
        // *empty word* is accepted, matching `a?`'s own language.
        assert!(reg.nullable());
    }

    #[test]
    fn dead_end_verdict_matches_documented_asymmetry() {
        use crate::analysis::Query;
        // Includes: left dead alone is enough; right dead alone is not.
        assert!(dead_end_verdict(Query::Includes, true, false));
        assert!(!dead_end_verdict(Query::Includes, false, true));
        // Equivalent: needs both.
        assert!(!dead_end_verdict(Query::Equivalent, true, false));
        assert!(dead_end_verdict(Query::Equivalent, true, true));
        // Overlap: either alone is enough.
        assert!(dead_end_verdict(Query::Overlap, true, false));
        assert!(dead_end_verdict(Query::Overlap, false, true));
    }
}
