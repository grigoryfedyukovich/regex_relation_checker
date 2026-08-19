//! Antimirov (partial-derivative) backend for regular-language relation checking.
//!
//! Brzozowski derivatives produce a *single* residual expression per character.
//! Antimirov partial derivatives produce a *finite set* of residuals (a linear
//! form). The union of their languages equals the Brzozowski residual language,
//! but the set representation maps more directly onto NFA states and often
//! stays smaller when alternation would otherwise duplicate structure inside
//! one big residual term.
//!
//! Decision procedure: product BFS over pairs of linear forms. A linear form
//! accepts when any member is nullable. Alphabet partitions are taken from the
//! first-sets of every residual in the current pair, same representative
//! policy as the other backends.
//!
//! References: Antimirov, "Partial Derivatives of Regular Expressions and
//! Finite Automaton Constructions" (Theoretical Computer Science, 1996).

use crate::analysis::{BackendResult, BackendStatus, Query, RelationBackend};
use crate::ast::{Expr, ExprKind};
use crate::charset::CharSet;
use crate::config::Config;
use crate::nfa::Nfa;
use crate::report::relation;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Residual algebra (shared shape with the Brzozowski backend)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
enum RegKind {
    Null,
    Eps,
    Atom(CharSet),
    Concat(Vec<Reg>),
    Alt(Vec<Reg>),
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
/// shared subterm repeatedly (e.g. as a `HashSet<Reg>` member during
/// dedup, or as part of a `(Reg, char)` cache key, both on the hot path
/// for wide linear forms) costs O(size) *every single time*, hit or miss.
fn compute_hash(kind: &RegKind) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    kind.hash(&mut hasher);
    hasher.finish()
}

#[derive(Clone, Debug)]
struct Reg(Rc<(RegKind, u64)>);

impl Reg {
    fn kind(&self) -> &RegKind {
        &self.0.as_ref().0
    }
    /// Build a `Reg` from a fresh `RegKind`, computing and caching its
    /// hash once. Every smart constructor below goes through this instead
    /// of `Self(Rc::new(...))` directly.
    fn wrap(kind: RegKind) -> Self {
        let h = compute_hash(&kind);
        Self(Rc::new((kind, h)))
    }

    fn null() -> Self {
        Self::wrap(RegKind::Null)
    }
    fn eps() -> Self {
        Self::wrap(RegKind::Eps)
    }
    fn atom(set: CharSet) -> Self {
        if set.is_empty() {
            Self::null()
        } else {
            Self::wrap(RegKind::Atom(set))
        }
    }
    fn star(inner: Reg) -> Self {
        match inner.kind() {
            RegKind::Null | RegKind::Eps => Self::eps(),
            RegKind::Star(_) => inner,
            _ => Self::wrap(RegKind::Star(inner)),
        }
    }
    fn concat(parts: Vec<Reg>) -> Self {
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
    fn alt(branches: Vec<Reg>) -> Self {
        let mut flat = Vec::new();
        for b in branches {
            match b.kind() {
                RegKind::Null => {}
                RegKind::Alt(inner) => flat.extend(inner.iter().cloned()),
                _ => flat.push(b),
            }
        }
        flat.sort_by(reg_ord);
        flat.dedup();
        match flat.len() {
            0 => Self::null(),
            1 => flat.pop().unwrap(),
            _ => Self::wrap(RegKind::Alt(flat)),
        }
    }

    fn nullable(&self) -> bool {
        match self.kind() {
            RegKind::Null | RegKind::Atom(_) => false,
            RegKind::Eps | RegKind::Star(_) => true,
            RegKind::Concat(parts) => parts.iter().all(Reg::nullable),
            RegKind::Alt(branches) => branches.iter().any(Reg::nullable),
        }
    }

    fn first_sets(&self, out: &mut Vec<CharSet>) {
        match self.kind() {
            RegKind::Null | RegKind::Eps => {}
            RegKind::Atom(set) => out.push(set.clone()),
            RegKind::Concat(parts) => {
                for p in parts {
                    p.first_sets(out);
                    if !p.nullable() {
                        break;
                    }
                }
            }
            RegKind::Alt(branches) => {
                for b in branches {
                    b.first_sets(out);
                }
            }
            RegKind::Star(inner) => inner.first_sets(out),
        }
    }

    /// Concatenate `self` on the left of `tail`, with normalization.
    fn then(self, tail: Reg) -> Reg {
        Reg::concat(vec![self, tail])
    }
}

impl std::hash::Hash for Reg {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
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

fn reg_ord(a: &Reg, b: &Reg) -> Ordering {
    use RegKind::*;
    fn rank(k: &RegKind) -> u8 {
        match k {
            Null => 0,
            Eps => 1,
            Atom(_) => 2,
            Concat(_) => 3,
            Alt(_) => 4,
            Star(_) => 5,
        }
    }
    let (ka, kb) = (a.kind(), b.kind());
    match rank(ka).cmp(&rank(kb)) {
        Ordering::Equal => {}
        o => return o,
    }
    match (ka, kb) {
        (Atom(sa), Atom(sb)) => {
            let ia = sa.intervals();
            let ib = sb.intervals();
            match ia.len().cmp(&ib.len()) {
                Ordering::Equal => {}
                o => return o,
            }
            for (x, y) in ia.iter().zip(ib.iter()) {
                match (x.start, x.end).cmp(&(y.start, y.end)) {
                    Ordering::Equal => {}
                    o => return o,
                }
            }
            Ordering::Equal
        }
        (Concat(pa), Concat(pb)) | (Alt(pa), Alt(pb)) => {
            // Length first, *then* elementwise -- confirmed via direct
            // instrumentation (not just bench pass/fail, which can't tell
            // "faster but still over budget" from "no effect") to be
            // where `from_parts`'s sort spends most of its time on chains
            // like 500 concatenated `a?`s: comparing two different-length
            // Concat chains built from the same repeated element walks
            // every shared element (all trivially equal) before ever
            // reaching the part that actually distinguishes them.
            match pa.len().cmp(&pb.len()) {
                Ordering::Equal => {}
                o => return o,
            }
            for (x, y) in pa.iter().zip(pb.iter()) {
                match reg_ord(x, y) {
                    Ordering::Equal => {}
                    o => return o,
                }
            }
            Ordering::Equal
        }
        (Star(ia), Star(ib)) => reg_ord(ia, ib),
        _ => Ordering::Equal,
    }
}

fn from_expr(expr: &Expr) -> Reg {
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

fn expand_repeat(inner: Reg, min: usize, max: Option<usize>) -> Reg {
    let mut parts = Vec::new();
    for _ in 0..min {
        parts.push(inner.clone());
    }
    match max {
        None => {
            parts.push(Reg::star(inner));
            Reg::concat(parts)
        }
        Some(maximum) => {
            for _ in min..maximum {
                parts.push(Reg::alt(vec![Reg::eps(), inner.clone()]));
            }
            Reg::concat(parts)
        }
    }
}

// ---------------------------------------------------------------------------
// Linear forms = finite sets of residuals (Antimirov)
// ---------------------------------------------------------------------------

/// Sorted, deduplicated set of residual expressions.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct LinearForm(Vec<Reg>);

impl LinearForm {
    fn empty() -> Self {
        Self(Vec::new())
    }

    fn singleton(r: Reg) -> Self {
        if matches!(r.kind(), RegKind::Null) {
            Self::empty()
        } else {
            Self(vec![r])
        }
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn nullable(&self) -> bool {
        self.0.iter().any(Reg::nullable)
    }

    fn first_sets(&self, out: &mut Vec<CharSet>) {
        for r in &self.0 {
            r.first_sets(out);
        }
    }

    /// Build a normalized linear form from raw, possibly-unsorted,
    /// possibly-duplicated, possibly-`Null` members in a single pass.
    ///
    /// Prefer this over folding many sources together with repeated
    /// [`LinearForm::union`] calls: each `union` re-sorts and re-dedups its
    /// accumulator from scratch, so combining `k` sources one at a time
    /// (as in `∂ₐ(E₁|E₂|…|Eₖ) = ∂ₐ(E₁) ∪ ∂ₐ(E₂) ∪ … ∪ ∂ₐ(Eₖ)`) costs
    /// `O(k² log k)`. Collecting every source's raw members into one `Vec`
    /// first and normalizing once is the point of batching in the first
    /// place -- this is exactly the case that matters for wide
    /// alternation, the pattern shape this backend exists to handle well
    /// (see the module doc comment).
    ///
    /// Deliberately hash-based (dedup first, sort only the survivors)
    /// rather than sort-then-dedup, though: `partial_der`'s `Concat` case
    /// is recursive, and each member of a wide linear form can itself
    /// expand into its own full downward-closed derivative set (this is
    /// the "`tail` recurs constantly" case the `Concat` case's own comment
    /// describes). For a chain of `k` nullable elements (`a?` repeated,
    /// with no wrapping `*`, is exactly this shape), the raw `parts`
    /// handed to this function can be `Θ(k^2)` in size even though the
    /// truly distinct count is `Θ(k)` -- almost all of it duplicates.
    /// Sorting that raw list first, the way this used to work, pays for
    /// `Θ(k^2 log k)` comparisons before `dedup()` ever gets to throw
    /// nearly all of them away. Hashing every element into a `HashSet`
    /// first collapses the duplicates in one amortized-O(1)-per-element
    /// pass (each element is still touched once, but never compared
    /// against arbitrarily many others the way a sort does), and only the
    /// true survivors -- `Θ(k)`, not `Θ(k^2)` -- ever reach the sort that
    /// gives this its canonical order.
    #[cfg(test)]
    fn from_parts(parts: Vec<Reg>) -> Self {
        let mut seen: HashSet<Reg> = HashSet::with_capacity(parts.len());
        for r in parts {
            if !matches!(r.kind(), RegKind::Null) {
                seen.insert(r);
            }
        }
        let mut deduped: Vec<Reg> = seen.into_iter().collect();
        deduped.sort_by(reg_ord);
        Self(deduped)
    }

    #[cfg(test)]
    fn union(mut self, other: LinearForm) -> Self {
        self.0.extend(other.0);
        Self::from_parts(self.0)
    }

    /// Right-concatenate every member with `tail`. Reference-only now (see
    /// `RawForm::then_all` for the production path).
    #[cfg(test)]
    fn then_all(self, tail: &Reg) -> Self {
        if matches!(tail.kind(), RegKind::Null) {
            return Self::empty();
        }
        if matches!(tail.kind(), RegKind::Eps) {
            return self;
        }
        let mut out = Vec::with_capacity(self.0.len());
        for r in self.0 {
            out.push(r.then(tail.clone()));
        }
        Self::from_parts(out)
    }
}

/// A lazily-combined multiset of residual terms, used internally while
/// `partial_der` recurses and `partial_der_form` aggregates a linear
/// form's members.
///
/// Building a union or a right-concatenation ("then_all") is O(1) here --
/// it just links `Rc`-shared subtrees together -- deferring the O(size)
/// work of actually visiting every member to `flatten()`, which callers
/// invoke exactly once, at the point they need a canonical, deduplicated,
/// sorted `LinearForm` for state-identity purposes. That's the point:
/// `LinearForm::union`/`then_all` (above) re-normalize their *entire*
/// accumulator on every call, which is correct but means a long recursive
/// unwind (the `Concat` case below, called once per member of a wide
/// state) pays that normalization cost once per level, and once per
/// member the level is reached from. When a state has ~k members and each
/// member's own derivative is itself ~k-sized (a `Star`-wrapped run of
/// k roughly-interchangeable elements is exactly this shape -- see
/// `mega_equivalent__antimirov-block-position-*`), that's k separate
/// members each independently re-normalizing a ~k-sized result: O(k^2)
/// raw element-touches minimum, confirmed by direct instrumentation
/// (a temporary `from_parts` call-size counter, not just wall-clock
/// timing) to be dominated by a *single* call receiving exactly k^2
/// elements -- not k calls of ~k each, one call of size k^2, from a wide
/// member unioning in another wide member's already-computed result and
/// having to copy every element of it out to do so.
///
/// `flatten()` memoizes by `Rc` pointer identity (not by value): a
/// sub-tree that's referenced from many places in the combined tree --
/// e.g. `partial_der(Star(inner), ch, cache)`'s cached result, pulled in
/// by every one of a wide state's members because each of them has that
/// same trailing `Star` branch -- is only ever walked once no matter how
/// many places link to it, because every one of those links is the *same*
/// `Rc` allocation (it came from the same cache entry, and `Rc::clone`
/// preserves pointer identity, so `Rc::as_ptr` is a sound identity key).
///
/// There is a second, *different* wide-state shape -- a run of `k`
/// distinct, individually-*cheap* members that don't share one large
/// sub-component (a flat `a+` repeated `k` times with no wrapping `*` is
/// exactly this: at any point the state has up to `k` members, one per
/// "which of the first `k` copies could still be absorbing extra `a`s",
/// but each member's own derivative is small on its own -- there's
/// nothing large being redundantly re-touched). Folding `k` such members
/// through repeated pairwise `union()` still costs only O(k) `Rc`
/// allocations *per state*, same asymptotic class as the old eager
/// `Vec`-based accumulation -- but O(k) *heap allocations* is a
/// meaningfully worse constant than the old approach's O(k) plain
/// `Vec::extend` pushes, and summed over the ~k states such a pattern
/// produces, that constant-factor gap is what a first version of this
/// type regressed on (confirmed by direct profiling against the
/// unmodified backend on the same pattern, not just wall-clock
/// comparison). `union_many` exists for exactly this: combining `k`
/// members costs one `Vec` allocation plus one `Rc::new` wrapping it,
/// not `k` separate ones, while `flatten()` still memoizes each element's
/// *own* shared sub-structure (if any) exactly as it would under `Union`.
#[derive(Clone)]
enum RawForm {
    Empty,
    Leaf(Reg),
    Union(Rc<RawForm>, Rc<RawForm>),
    /// Combines many parts in one allocation rather than folding them
    /// pairwise through `Union`; see the type's own doc comment above.
    UnionMany(Rc<Vec<RawForm>>),
    /// Every eventual leaf reachable through `Rc<RawForm>` should be
    /// right-concatenated with `Reg` once flattened. Kept as an explicit
    /// node (rather than eagerly mapped over the subtree) so building it
    /// stays O(1); see `flatten`'s `combined_tail` for how nested
    /// `ThenAll`s compose correctly when finally applied.
    ThenAll(Rc<RawForm>, Reg),
}

impl RawForm {
    fn empty() -> Self {
        RawForm::Empty
    }

    fn singleton(r: Reg) -> Self {
        if matches!(r.kind(), RegKind::Null) {
            RawForm::Empty
        } else {
            RawForm::Leaf(r)
        }
    }

    fn union(self, other: RawForm) -> Self {
        match (&self, &other) {
            (RawForm::Empty, _) => other,
            (_, RawForm::Empty) => self,
            _ => RawForm::Union(Rc::new(self), Rc::new(other)),
        }
    }

    /// Combine many parts in one allocation. See the type's own doc
    /// comment for why this exists alongside `union`: folding `k` parts
    /// through repeated pairwise `union()` calls costs `k` separate `Rc`
    /// allocations; this costs one, regardless of `k`.
    fn union_many(parts: Vec<RawForm>) -> Self {
        let mut filtered: Vec<RawForm> = Vec::with_capacity(parts.len());
        for p in parts {
            if !matches!(p, RawForm::Empty) {
                filtered.push(p);
            }
        }
        match filtered.len() {
            0 => RawForm::Empty,
            1 => filtered.pop().unwrap(),
            _ => RawForm::UnionMany(Rc::new(filtered)),
        }
    }

    /// Right-concatenate every eventual leaf with `tail`. Matches
    /// `LinearForm::then_all`'s short-circuiting exactly (`Null` tail
    /// collapses everything to empty; `Eps` tail is a no-op) so a
    /// `RawForm` built this way flattens to the same result
    /// `LinearForm::then_all` would have produced.
    fn then_all(self, tail: &Reg) -> Self {
        match tail.kind() {
            RegKind::Null => RawForm::Empty,
            RegKind::Eps => self,
            _ => match self {
                RawForm::Empty => RawForm::Empty,
                other => RawForm::ThenAll(Rc::new(other), tail.clone()),
            },
        }
    }

    /// Canonicalize into a `LinearForm`: dedupe, drop any stray `Null`s,
    /// sort. Semantically equivalent to eagerly `union`/`then_all`-ing
    /// through `LinearForm` the whole way and calling `from_parts` once
    /// at the end, but visits each *distinct* (by `Rc` pointer, under a
    /// given composed pending tail) subtree only once regardless of how
    /// many places in the tree link to it.
    fn flatten(&self) -> LinearForm {
        // Memo key: (subtree pointer, the single Reg that composes every
        // `ThenAll` wrapping encountered on the path from the flatten
        // root down to this subtree so far). Composing pending tails into
        // one `Reg` via `Reg::concat` -- rather than carrying a `Vec<Reg>`
        // -- works because `Reg::concat` is associative/flattening:
        // `x.then(a).then(b)` and `x.then(Reg::concat([a, b]))` normalize
        // to the same term. That collapses the memo key to something
        // `Reg`'s already-O(1) `Hash`/`Eq` handles directly, and -- for
        // this backend's motivating shape -- every reference to a shared
        // subtree like `S = partial_der(Star(_), ch, cache)` arrives with
        // the *same* composed tail (`Eps`, i.e. unioned in untransformed),
        // so the very first visit's memo entry is reused by every other
        // member that pulls the same `S` in, which is exactly the
        // redundant work this type exists to avoid.
        let mut memo_eps: HashSet<usize> = HashSet::new();
        let mut memo_tailed: HashSet<(usize, Reg)> = HashSet::new();
        let mut seen: HashSet<Reg> = HashSet::new();
        fn visit(
            ptr: usize,
            combined_tail: &Reg,
            memo_eps: &mut HashSet<usize>,
            memo_tailed: &mut HashSet<(usize, Reg)>,
        ) -> bool {
            if matches!(combined_tail.kind(), RegKind::Eps) {
                memo_eps.insert(ptr)
            } else {
                memo_tailed.insert((ptr, combined_tail.clone()))
            }
        }
        fn walk(
            node: &RawForm,
            combined_tail: &Reg,
            memo_eps: &mut HashSet<usize>,
            memo_tailed: &mut HashSet<(usize, Reg)>,
            seen: &mut HashSet<Reg>,
        ) {
            match node {
                RawForm::Empty => {}
                RawForm::Leaf(r) => {
                    let applied = if matches!(combined_tail.kind(), RegKind::Eps) {
                        r.clone()
                    } else {
                        r.clone().then(combined_tail.clone())
                    };
                    if !matches!(applied.kind(), RegKind::Null) {
                        seen.insert(applied);
                    }
                }
                RawForm::Union(a, b) => {
                    if visit(Rc::as_ptr(a) as usize, combined_tail, memo_eps, memo_tailed) {
                        walk(a, combined_tail, memo_eps, memo_tailed, seen);
                    }
                    if visit(Rc::as_ptr(b) as usize, combined_tail, memo_eps, memo_tailed) {
                        walk(b, combined_tail, memo_eps, memo_tailed, seen);
                    }
                }
                RawForm::UnionMany(parts) => {
                    // Memoize the whole group by its own Rc<Vec<_>>
                    // pointer -- if this exact group is reached again
                    // (same allocation, same pending tail), every one of
                    // its members' contributions is already in `seen`.
                    // Each individual member's *own* internal sharing
                    // (e.g. a ThenAll wrapping some large cached S) still
                    // memoizes correctly when recursed into below, since
                    // that sharing lives at the member's own Rc-wrapped
                    // sub-structure, independent of how this group itself
                    // is stored.
                    if visit(
                        Rc::as_ptr(parts) as usize,
                        combined_tail,
                        memo_eps,
                        memo_tailed,
                    ) {
                        for p in parts.iter() {
                            walk(p, combined_tail, memo_eps, memo_tailed, seen);
                        }
                    }
                }
                RawForm::ThenAll(inner, tail) => {
                    let new_combined = if matches!(combined_tail.kind(), RegKind::Eps) {
                        tail.clone()
                    } else {
                        Reg::concat(vec![tail.clone(), combined_tail.clone()])
                    };
                    if visit(
                        Rc::as_ptr(inner) as usize,
                        &new_combined,
                        memo_eps,
                        memo_tailed,
                    ) {
                        walk(inner, &new_combined, memo_eps, memo_tailed, seen);
                    }
                }
            }
        }
        walk(
            self,
            &Reg::eps(),
            &mut memo_eps,
            &mut memo_tailed,
            &mut seen,
        );
        let mut deduped: Vec<Reg> = seen.into_iter().collect();
        deduped.sort_by(reg_ord);
        LinearForm(deduped)
    }
}

/// Antimirov partial derivative: finite set of residuals.
/// Reference implementation, kept for cross-checking `partial_der`/
/// `partial_der_form` (below) against: eagerly normalizes at every
/// combining step rather than deferring to a single memoized `flatten()`.
/// Correct, just not what production code paths call anymore -- see
/// `partial_der_matches_naive_reference` and `RawForm`'s doc comment for
/// why the two can still disagree on *speed* but must never disagree on
/// *result*.
#[cfg(test)]
fn partial_der_naive(
    r: &Reg,
    ch: char,
    cache: &mut HashMap<(Reg, char), LinearForm>,
) -> LinearForm {
    if let Some(cached) = cache.get(&(r.clone(), ch)) {
        return cached.clone();
    }
    let result = match r.kind() {
        RegKind::Null | RegKind::Eps => LinearForm::empty(),
        RegKind::Atom(set) => {
            if set.contains(ch) {
                LinearForm::singleton(Reg::eps())
            } else {
                LinearForm::empty()
            }
        }
        RegKind::Alt(branches) => {
            let mut parts = Vec::new();
            for b in branches {
                parts.extend(partial_der_naive(b, ch, cache).0);
            }
            LinearForm::from_parts(parts)
        }
        RegKind::Concat(parts) => {
            if parts.is_empty() {
                LinearForm::empty()
            } else {
                let head = &parts[0];
                let tail = if parts.len() == 1 {
                    Reg::eps()
                } else {
                    Reg::concat(parts[1..].to_vec())
                };
                let mut acc = partial_der_naive(head, ch, cache).then_all(&tail);
                if head.nullable() {
                    acc = acc.union(partial_der_naive(&tail, ch, cache));
                }
                acc
            }
        }
        RegKind::Star(inner) => partial_der_naive(inner, ch, cache).then_all(r),
    };
    cache.insert((r.clone(), ch), result.clone());
    result
}

#[cfg(test)]
fn partial_der_form_naive(
    form: &LinearForm,
    ch: char,
    cache: &mut HashMap<(Reg, char), LinearForm>,
) -> LinearForm {
    let mut parts = Vec::new();
    for r in &form.0 {
        parts.extend(partial_der_naive(r, ch, cache).0);
    }
    LinearForm::from_parts(parts)
}

/// Antimirov partial derivative: finite set of residuals, built as a
/// lazily-combined `RawForm`. Callers that need a canonical `LinearForm`
/// -- for state identity, nullability, first-sets -- call `.flatten()`
/// once they're done combining, rather than this function normalizing on
/// every recursive step; see `RawForm`'s doc comment for why that
/// distinction is the whole point. Semantically identical to
/// `partial_der_naive` above -- `partial_der_matches_naive_reference`
/// checks exactly that, across a range of shapes, on every test run.
fn partial_der(r: &Reg, ch: char, cache: &mut HashMap<(Reg, char), RawForm>) -> RawForm {
    if let Some(cached) = cache.get(&(r.clone(), ch)) {
        return cached.clone();
    }
    let result = match r.kind() {
        RegKind::Null | RegKind::Eps => RawForm::empty(),
        RegKind::Atom(set) => {
            if set.contains(ch) {
                RawForm::singleton(Reg::eps())
            } else {
                RawForm::empty()
            }
        }
        RegKind::Alt(branches) => {
            // ∂ₐ(E₁|…|Eₖ) = ∂ₐ(E₁) ∪ … ∪ ∂ₐ(Eₖ). `union_many` costs one
            // allocation for the whole group, not k separate ones the
            // way folding branch-by-branch through `union` would -- see
            // `RawForm`'s doc comment for why that distinction matters.
            RawForm::union_many(branches.iter().map(|b| partial_der(b, ch, cache)).collect())
        }
        RegKind::Concat(parts) => {
            // ∂(e1 e2 … ek) = ∂(e1)·(e2…ek) ∪ ν(e1)·∂(e2…ek)
            if parts.is_empty() {
                RawForm::empty()
            } else {
                let head = &parts[0];
                let tail = if parts.len() == 1 {
                    Reg::eps()
                } else {
                    Reg::concat(parts[1..].to_vec())
                };
                let mut acc = partial_der(head, ch, cache).then_all(&tail);
                if head.nullable() {
                    acc = acc.union(partial_der(&tail, ch, cache));
                }
                acc
            }
        }
        RegKind::Star(inner) => {
            // ∂(e*) = ∂(e)·e*
            partial_der(inner, ch, cache).then_all(r)
        }
    };
    cache.insert((r.clone(), ch), result.clone());
    result
}

fn partial_der_form(
    form: &LinearForm,
    ch: char,
    cache: &mut HashMap<(Reg, char), RawForm>,
) -> LinearForm {
    RawForm::union_many(form.0.iter().map(|r| partial_der(r, ch, cache)).collect()).flatten()
}

fn representative_chars(sets: &[CharSet]) -> Vec<char> {
    let mut boundaries: Vec<u32> = Vec::new();
    for set in sets {
        for interval in set.intervals() {
            boundaries.push(interval.start);
            if interval.end < 0x10ffff {
                boundaries.push(interval.end + 1);
            }
        }
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    boundaries
        .into_iter()
        .filter_map(char::from_u32)
        .filter(|ch| sets.iter().any(|set| set.contains(*ch)))
        .collect()
}

fn classify_relation(
    query: Query,
    left_accepts: bool,
    right_accepts: bool,
) -> Option<&'static str> {
    match query {
        Query::Overlap if left_accepts && right_accepts => Some(relation::IN_BOTH),
        Query::Includes if left_accepts && !right_accepts => Some(relation::LEFT_ONLY),
        Query::Equivalent if left_accepts && !right_accepts => Some(relation::LEFT_ONLY),
        Query::Equivalent if !left_accepts && right_accepts => Some(relation::RIGHT_ONLY),
        _ => None,
    }
}

/// Empty linear form is the absorbing dead language on that side.
fn is_dead_end(query: Query, left: &LinearForm, right: &LinearForm) -> bool {
    let left_dead = left.is_empty();
    let right_dead = right.is_empty();
    match query {
        Query::Overlap => left_dead || right_dead,
        Query::Includes => left_dead,
        Query::Equivalent => left_dead && right_dead,
        Query::Empty | Query::Match => false,
    }
}

fn stopped_result(
    status: BackendStatus,
    visited: usize,
    generated: usize,
    started: Instant,
) -> BackendResult {
    BackendResult {
        status,
        witness: None,
        relation: None,
        visited_states: visited,
        generated_transitions: generated,
        analysis_ms: started.elapsed().as_millis(),
        witness_extraction_ms: 0,
    }
}

fn search_product(query: Query, left: Reg, right: Reg, config: &Config) -> BackendResult {
    let started = Instant::now();
    let deadline = Duration::from_millis(config.timeout_ms);

    #[derive(Clone, Debug, Eq, PartialEq, Hash)]
    struct ProductKey {
        left: LinearForm,
        right: LinearForm,
    }
    struct SearchNode {
        key: ProductKey,
        parent: Option<(usize, char)>,
    }

    let initial = ProductKey {
        left: LinearForm::singleton(left),
        right: LinearForm::singleton(right),
    };
    let mut nodes = vec![SearchNode {
        key: initial.clone(),
        parent: None,
    }];
    let mut visited = HashSet::new();
    visited.insert(initial);
    let mut queue = VecDeque::from([0usize]);
    let mut generated_transitions = 0usize;
    let mut left_cache: HashMap<(LinearForm, char), LinearForm> = HashMap::new();
    let mut right_cache: HashMap<(LinearForm, char), LinearForm> = HashMap::new();
    // Persists for the whole query, across every BFS step -- not just
    // within one partial_der_form call. The same residual term (e.g. "k
    // remaining copies of a?") recurs both as a direct linear-form member
    // at one step and as a recursive sub-computation while deriving a
    // longer member at another step; without this, each of those O(n)
    // occurrences repeats its own O(size) derivation independently.
    // `RawForm`-valued (not `LinearForm`): a cache *hit* here is now O(1)
    // (an `Rc` clone) rather than O(size of the cached form), which is
    // what actually matters once a wide state's members start pulling
    // the same large cached sub-result in repeatedly -- see `RawForm`'s
    // doc comment.
    let mut term_cache: HashMap<(Reg, char), RawForm> = HashMap::new();

    while let Some(node_id) = queue.pop_front() {
        if started.elapsed() >= deadline {
            return stopped_result(
                BackendStatus::Timeout,
                nodes.len(),
                generated_transitions,
                started,
            );
        }
        let left_acc = nodes[node_id].key.left.nullable();
        let right_acc = nodes[node_id].key.right.nullable();
        if let Some(rel) = classify_relation(query, left_acc, right_acc) {
            let analysis_ms = started.elapsed().as_millis();
            let witness_started = Instant::now();
            let mut reversed = Vec::new();
            let mut cur = node_id;
            while let Some((parent, ch)) = nodes[cur].parent {
                reversed.push(ch);
                cur = parent;
            }
            reversed.reverse();
            let witness: String = reversed.into_iter().collect();
            return BackendResult {
                status: BackendStatus::Found,
                witness: Some(witness),
                relation: Some(rel.to_owned()),
                visited_states: nodes.len(),
                generated_transitions,
                analysis_ms,
                witness_extraction_ms: witness_started.elapsed().as_millis(),
            };
        }

        if is_dead_end(query, &nodes[node_id].key.left, &nodes[node_id].key.right) {
            continue;
        }

        let mut first = Vec::new();
        nodes[node_id].key.left.first_sets(&mut first);
        nodes[node_id].key.right.first_sets(&mut first);
        for ch in representative_chars(&first) {
            // Checked per transition, not just once per popped node: a
            // single node can have a large fan-out, and that whole batch
            // would otherwise run to completion before the next chance to
            // notice the deadline has passed.
            if started.elapsed() >= deadline {
                return stopped_result(
                    BackendStatus::Timeout,
                    nodes.len(),
                    generated_transitions,
                    started,
                );
            }
            generated_transitions += 1;
            let next_left = match left_cache.get(&(nodes[node_id].key.left.clone(), ch)) {
                Some(v) => v.clone(),
                None => {
                    let computed = partial_der_form(&nodes[node_id].key.left, ch, &mut term_cache);
                    left_cache.insert((nodes[node_id].key.left.clone(), ch), computed.clone());
                    computed
                }
            };
            let next_right = match right_cache.get(&(nodes[node_id].key.right.clone(), ch)) {
                Some(v) => v.clone(),
                None => {
                    let computed = partial_der_form(&nodes[node_id].key.right, ch, &mut term_cache);
                    right_cache.insert((nodes[node_id].key.right.clone(), ch), computed.clone());
                    computed
                }
            };
            let next = ProductKey {
                left: next_left,
                right: next_right,
            };
            if !visited.insert(next.clone()) {
                continue;
            }
            if nodes.len() >= config.max_product_states {
                return stopped_result(
                    BackendStatus::StateLimit,
                    nodes.len(),
                    generated_transitions,
                    started,
                );
            }
            let next_id = nodes.len();
            nodes.push(SearchNode {
                key: next,
                parent: Some((node_id, ch)),
            });
            queue.push_back(next_id);
        }
    }

    stopped_result(
        BackendStatus::Exhausted,
        nodes.len(),
        generated_transitions,
        started,
    )
}

fn search_single(reg: Reg, config: &Config) -> BackendResult {
    let started = Instant::now();
    let deadline = Duration::from_millis(config.timeout_ms);

    #[derive(Clone, Debug, Eq, PartialEq, Hash)]
    struct Key(LinearForm);
    struct Node {
        key: Key,
        parent: Option<(usize, char)>,
    }

    let initial = Key(LinearForm::singleton(reg));
    let mut nodes = vec![Node {
        key: initial.clone(),
        parent: None,
    }];
    let mut visited = HashSet::new();
    visited.insert(initial);
    let mut queue = VecDeque::from([0usize]);
    let mut generated_transitions = 0usize;
    let mut term_cache: HashMap<(Reg, char), RawForm> = HashMap::new();

    while let Some(node_id) = queue.pop_front() {
        if started.elapsed() >= deadline {
            return stopped_result(
                BackendStatus::Timeout,
                nodes.len(),
                generated_transitions,
                started,
            );
        }
        if nodes[node_id].key.0.nullable() {
            let analysis_ms = started.elapsed().as_millis();
            let witness_started = Instant::now();
            let mut reversed = Vec::new();
            let mut cur = node_id;
            while let Some((parent, ch)) = nodes[cur].parent {
                reversed.push(ch);
                cur = parent;
            }
            reversed.reverse();
            let witness: String = reversed.into_iter().collect();
            return BackendResult {
                status: BackendStatus::Found,
                witness: Some(witness),
                relation: Some(relation::IN_LANGUAGE.to_owned()),
                visited_states: nodes.len(),
                generated_transitions,
                analysis_ms,
                witness_extraction_ms: witness_started.elapsed().as_millis(),
            };
        }
        if nodes[node_id].key.0.is_empty() {
            continue;
        }
        let mut first = Vec::new();
        nodes[node_id].key.0.first_sets(&mut first);
        for ch in representative_chars(&first) {
            // See the matching comment in `search_product` above.
            if started.elapsed() >= deadline {
                return stopped_result(
                    BackendStatus::Timeout,
                    nodes.len(),
                    generated_transitions,
                    started,
                );
            }
            generated_transitions += 1;
            let next = Key(partial_der_form(&nodes[node_id].key.0, ch, &mut term_cache));
            if !visited.insert(next.clone()) {
                continue;
            }
            if nodes.len() >= config.max_product_states {
                return stopped_result(
                    BackendStatus::StateLimit,
                    nodes.len(),
                    generated_transitions,
                    started,
                );
            }
            let next_id = nodes.len();
            nodes.push(Node {
                key: next,
                parent: Some((node_id, ch)),
            });
            queue.push_back(next_id);
        }
    }

    stopped_result(
        BackendStatus::Exhausted,
        nodes.len(),
        generated_transitions,
        started,
    )
}

/// Antimirov partial-derivative engine.
pub struct AntimirovBackend;

impl RelationBackend for AntimirovBackend {
    fn name(&self) -> &'static str {
        "antimirov"
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn analyze_binary(
        &self,
        _query: Query,
        _left: &Nfa,
        _right: &Nfa,
        _config: &Config,
    ) -> BackendResult {
        // Prefer the AST-aware path; NFA-only entry is unused for this
        // backend. `Timeout` (never `Exhausted`) signals "did not actually
        // run" -- `Exhausted` would read as a completed, decisive search
        // that legitimately found no witness, which is not what happened
        // here. Matches `DerivativeBackend`'s identical fallback.
        BackendResult {
            status: BackendStatus::Timeout,
            witness: None,
            relation: None,
            visited_states: 0,
            generated_transitions: 0,
            analysis_ms: 0,
            witness_extraction_ms: 0,
        }
    }

    fn analyze_empty(&self, _nfa: &Nfa, _config: &Config) -> BackendResult {
        BackendResult {
            status: BackendStatus::Timeout,
            witness: None,
            relation: None,
            visited_states: 0,
            generated_transitions: 0,
            analysis_ms: 0,
            witness_extraction_ms: 0,
        }
    }

    fn analyze_binary_expr(
        &self,
        query: Query,
        left_expr: &Expr,
        right_expr: &Expr,
        _left: &Nfa,
        _right: &Nfa,
        config: &Config,
    ) -> BackendResult {
        search_product(query, from_expr(left_expr), from_expr(right_expr), config)
    }

    fn analyze_empty_expr(&self, expr: &Expr, _nfa: &Nfa, config: &Config) -> BackendResult {
        search_single(from_expr(expr), config)
    }

    fn match_input(&self, expr: &Expr, _nfa: &Nfa, input: &str, config: &Config) -> BackendResult {
        match_antimirov(expr, input, config)
    }
}

/// A tiny lazily-built DFA over `LinearForm` states, keyed by small integer
/// ids rather than by the linear forms themselves.
///
/// `match_antimirov` used to cache transitions as `HashMap<(LinearForm,
/// char), LinearForm>`: that avoids *recomputing* an already-seen
/// transition, but every lookup still has to *hash* the current
/// `LinearForm` to find it -- and hashing one costs O(total size of its
/// member residuals), since a `LinearForm` is a `Vec<Reg>` and each `Reg`
/// hashes its whole pointed-to tree. For patterns whose distinct
/// linear-form *count* stays small but whose individual member terms stay
/// large -- wide alternation wrapped in an outer repetition, exactly the
/// shape `docs/backends.md`'s antimirov section describes as this
/// backend's target case -- that made every step of a long match walk pay
/// a cost proportional to term size, even on a cache hit. That's a real
/// gap: precisely the patterns this backend is supposed to be small for
/// (few distinct linear forms) were paying full price on every step of a
/// walk anyway.
///
/// Interning pays the hashing cost once per *distinct* linear form, the
/// first time it's seen; every revisit after that is a
/// `(usize, char) -> usize` lookup, cheap regardless of how large the
/// underlying linear form is. This is the same "lazy DFA" technique real
/// derivative-based regex engines use to make repeated/long matching fast
/// without a separate upfront determinization pass -- see the identical
/// structure in `derivative.rs`'s `ResidualInterner`.
struct LinearFormInterner {
    ids: HashMap<LinearForm, usize>,
    states: Vec<LinearForm>,
}

impl LinearFormInterner {
    fn new(start: LinearForm) -> Self {
        let mut ids = HashMap::new();
        ids.insert(start.clone(), 0);
        Self {
            ids,
            states: vec![start],
        }
    }

    /// The id for `state`, assigning a new one if it's not already known
    /// and there's room under `max_states`. Only returns `None` when
    /// `state` is genuinely new *and* the limit is already reached --
    /// revisiting an already-known state never fails, even exactly at the
    /// limit, since it doesn't grow the state count.
    fn intern(&mut self, state: LinearForm, max_states: usize) -> Option<usize> {
        if let Some(&id) = self.ids.get(&state) {
            return Some(id);
        }
        if self.states.len() >= max_states {
            return None;
        }
        let id = self.states.len();
        self.states.push(state.clone());
        self.ids.insert(state, id);
        Some(id)
    }
}

fn match_antimirov(expr: &Expr, input: &str, config: &Config) -> BackendResult {
    let started = Instant::now();
    let deadline = Duration::from_millis(config.timeout_ms);
    let mut interner = LinearFormInterner::new(LinearForm::singleton(from_expr(expr)));
    // Finer-grained than `transitions` below: caches individual `Reg`
    // partial derivatives, which different interned `LinearForm` states
    // can still share as members even when the states themselves differ,
    // so it keeps paying for itself across states, not just within
    // repeated visits to the same one.
    let mut pd_cache: HashMap<(Reg, char), RawForm> = HashMap::new();
    let mut transitions: HashMap<(usize, char), usize> = HashMap::new();
    let mut current = 0usize;
    let mut generated = 0usize;

    for ch in input.chars() {
        if started.elapsed() >= deadline {
            return BackendResult {
                status: BackendStatus::Timeout,
                witness: None,
                relation: None,
                visited_states: interner.states.len(),
                generated_transitions: generated,
                analysis_ms: started.elapsed().as_millis(),
                witness_extraction_ms: 0,
            };
        }
        generated += 1;
        let next = match transitions.get(&(current, ch)) {
            Some(&id) => id,
            None => {
                let next_form = partial_der_form(&interner.states[current], ch, &mut pd_cache);
                match interner.intern(next_form, config.max_product_states) {
                    Some(id) => {
                        transitions.insert((current, ch), id);
                        id
                    }
                    None => {
                        return BackendResult {
                            status: BackendStatus::StateLimit,
                            witness: None,
                            relation: None,
                            visited_states: interner.states.len(),
                            generated_transitions: generated,
                            analysis_ms: started.elapsed().as_millis(),
                            witness_extraction_ms: 0,
                        };
                    }
                }
            }
        };
        current = next;
        if interner.states[current].is_empty() {
            break;
        }
    }

    let analysis_ms = started.elapsed().as_millis();
    let form = &interner.states[current];
    if form.nullable() {
        BackendResult {
            status: BackendStatus::Found,
            witness: Some(input.to_owned()),
            relation: Some(relation::IN_LANGUAGE.to_owned()),
            visited_states: interner.states.len(),
            generated_transitions: generated,
            analysis_ms,
            witness_extraction_ms: 0,
        }
    } else {
        BackendResult {
            status: BackendStatus::Exhausted,
            witness: None,
            relation: None,
            visited_states: interner.states.len(),
            generated_transitions: generated,
            analysis_ms,
            witness_extraction_ms: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{
        analyze_binary_with_backend, analyze_empty_with_backend, analyze_match_with_backend,
        AutomataBackend,
    };
    use crate::report::Verdict;
    use crate::{parse, Config};

    /// Walks `pattern`'s partial-derivative automaton `steps` characters
    /// deep (trying every character in `alphabet` at each state, not just
    /// one path), asserting at *every* single state along the way that the
    /// new lazy (`RawForm` + `flatten()`) computation agrees exactly with
    /// `partial_der_naive`'s eager one.
    ///
    /// This exists because a bug in how `RawForm::flatten` composes nested
    /// `ThenAll` wrappings (the tail-application-order logic) wouldn't
    /// necessarily show up on a single derivative step from a fresh
    /// pattern -- nesting comes from a `Concat`/`Star` combination
    /// recursing into its own already-`ThenAll`-wrapped sub-results, which
    /// several steps into a walk is exactly when it starts happening. Full
    /// BFS-equivalent coverage (every reachable state, not just one path)
    /// rather than a single spot-check.
    fn assert_lazy_matches_naive_along_walk(pattern: &str, alphabet: &[char], steps: usize) {
        let config = Config::default();
        let expr = parse(pattern, &config).expect("pattern parses");
        let start = from_expr(&expr);

        let mut lazy_cache: HashMap<(Reg, char), RawForm> = HashMap::new();
        let mut naive_cache: HashMap<(Reg, char), LinearForm> = HashMap::new();

        // BFS over LinearForm states reached via the *naive* path, cross
        // checking the lazy path agrees at every single one -- rather than
        // trusting the lazy path's own notion of "reachable" (which, if
        // flatten() were unsound, could itself be wrong).
        let mut frontier: Vec<LinearForm> = vec![LinearForm::singleton(start)];
        let mut seen: HashSet<LinearForm> = frontier.iter().cloned().collect();
        for step in 0..steps {
            let mut next_frontier = Vec::new();
            for state in &frontier {
                for &ch in alphabet {
                    let lazy_next = partial_der_form(state, ch, &mut lazy_cache);
                    let naive_next = partial_der_form_naive(state, ch, &mut naive_cache);
                    assert_eq!(
                        lazy_next, naive_next,
                        "pattern {:?}, step {}, char {:?}: lazy result diverged from naive reference\n  from state: {:?}",
                        pattern, step, ch, state
                    );
                    if seen.insert(naive_next.clone()) {
                        next_frontier.push(naive_next);
                    }
                }
            }
            if next_frontier.is_empty() {
                break;
            }
            frontier = next_frontier;
        }
    }

    #[test]
    fn lazy_partial_der_matches_naive_reference_across_pattern_shapes() {
        let alphabet = ['a', 'b', 'c'];
        // Deliberately covers: a flat nullable chain with no wrapping
        // star (the `mega_equivalent__*-optional` shape); the star-wrapped
        // repeated-block shape that motivated `RawForm` in the first
        // place (`mega_equivalent__antimirov-block-position-*`); nested
        // stars; wide alternation; mixed counted/optional/star nesting;
        // and a couple of small/degenerate cases (single atom, epsilon-ish
        // patterns) as sanity anchors.
        let cases: &[(&str, usize)] = &[
            ("a", 3),
            ("a?", 5),
            ("(a?){12}", 8),
            ("(((a|b|c*)?){12})*", 8),
            ("(a|b|c)*", 6),
            ("((a|b){4}c){3}x", 10),
            ("a+a+a+a+a+", 8),
            ("(a*)*", 5),
            ("(a|b|c|d|e|f|g)*", 5),
            ("(a?b?c?){6}", 8),
            ("a{2,5}", 6),
        ];
        for (pattern, steps) in cases {
            assert_lazy_matches_naive_along_walk(pattern, &alphabet, *steps);
        }
    }

    fn both_agree(query: Query, left: &str, right: &str) {
        let config = Config::default();
        let a = analyze_binary_with_backend(query, left, right, &config, &AutomataBackend).unwrap();
        let b =
            analyze_binary_with_backend(query, left, right, &config, &AntimirovBackend).unwrap();
        assert_eq!(
            a.verdict, b.verdict,
            "disagree on {:?}({:?}, {:?}): automata={:?} antimirov={:?}",
            query, left, right, a.verdict, b.verdict
        );
    }

    #[test]
    fn agrees_on_core_relations() {
        both_agree(Query::Equivalent, "a|b", "[ab]");
        both_agree(Query::Equivalent, "a+", "aa*");
        both_agree(Query::Overlap, "a+b", "ab+");
        both_agree(Query::Includes, "a+", "a*");
        both_agree(Query::Includes, "a*", "a+");
        both_agree(Query::Overlap, "a+", "b+");
        both_agree(Query::Equivalent, "(a|b)*", "(a*b*)*");
        both_agree(Query::Equivalent, "a{2,4}", "aa|aaa|aaaa");
    }

    #[test]
    fn empty_language_cases() {
        let config = Config::default();
        let r = analyze_empty_with_backend("a", &config, &AntimirovBackend).unwrap();
        assert_eq!(r.verdict, Verdict::No);
        let r = analyze_empty_with_backend("[^\\d\\D]", &config, &AntimirovBackend).unwrap();
        // may be Yes if class empty
        assert!(matches!(r.verdict, Verdict::Yes | Verdict::No));
    }

    #[test]
    fn partial_der_atom() {
        let a = Reg::atom(CharSet::singleton('a'));
        let mut cache = HashMap::new();
        let d = partial_der(&a, 'a', &mut cache).flatten();
        assert!(d.nullable());
        let d2 = partial_der(&a, 'b', &mut cache).flatten();
        assert!(d2.is_empty());
    }

    #[test]
    fn partial_der_star() {
        let star = Reg::star(Reg::atom(CharSet::singleton('a')));
        let mut cache = HashMap::new();
        let d = partial_der(&star, 'a', &mut cache).flatten();
        assert!(!d.is_empty());
        // a* after reading a is still nullable (ε member via a* )
        assert!(d.nullable());
    }

    #[test]
    fn partial_der_wide_alt_matches_naive_reference() {
        // `partial_der`'s `Alt` case now folds branch results together
        // through `RawForm::union` (O(1) per branch, since union is lazy)
        // rather than batching into one `Vec` and normalizing once --
        // check that still agrees with `partial_der_naive`, which does
        // the batch-then-normalize the old way, on a wide alternation.
        let branches: Vec<Reg> = "abcdefg"
            .chars()
            .map(|c| Reg::atom(CharSet::singleton(c)))
            .collect();
        let wide_alt = Reg::alt(branches);
        let mut cache = HashMap::new();
        let lazy = partial_der(&wide_alt, 'd', &mut cache).flatten();
        let mut naive_cache = HashMap::new();
        let naive = partial_der_naive(&wide_alt, 'd', &mut naive_cache);
        assert_eq!(lazy, naive);
        assert!(lazy.nullable(), "∂_d matches the 'd' branch -> eps");
    }

    #[test]
    fn respects_state_limit() {
        // Mirrors `derivative::tests::respects_state_limit`: a query that
        // genuinely needs expansion beyond the start state must report
        // `Unknown` once `max_product_states` is exhausted.
        let config = Config {
            max_product_states: 1,
            ..Config::default()
        };
        let report =
            analyze_binary_with_backend(Query::Equivalent, "a+", "b+", &config, &AntimirovBackend)
                .unwrap();
        assert_eq!(report.verdict, Verdict::Unknown);
    }

    #[test]
    fn immediate_decision_survives_tight_state_limit() {
        // Regression guard: a query decidable *at the start state* (zero
        // expansions needed) must still return its answer even when
        // `max_product_states` is as tight as 1. `search_product` and
        // `search_single` only push a *new* node once they've confirmed
        // room for it; they must not also refuse to look at the state
        // that's already on the frontier.
        let config = Config {
            max_product_states: 1,
            ..Config::default()
        };
        // Overlap: 'a*' and 'b*' both accept the empty string, so the verdict
        // is decidable from the initial product state alone.
        let report =
            analyze_binary_with_backend(Query::Overlap, "a*", "b*", &config, &AntimirovBackend)
                .unwrap();
        assert_eq!(report.verdict, Verdict::Yes);
        assert_eq!(report.witness.unwrap().value, "");

        // Empty: 'a*' accepts the empty string immediately, i.e. its
        // language is *not* empty, so the correct verdict for the `empty`
        // query is `No` (this query's polarity is inverted from the others:
        // `Yes` means "yes, this language is empty" -- see the pre-existing
        // `empty_language_cases` test above, which the first version of
        // this test failed to follow).
        let report = analyze_empty_with_backend("a*", &config, &AntimirovBackend).unwrap();
        assert_eq!(report.verdict, Verdict::No);
        assert_eq!(report.witness.unwrap().value, "");
    }

    #[test]
    fn match_walk_revisiting_a_known_state_survives_a_tight_state_limit() {
        // Regression guard for `LinearFormInterner::intern`: revisiting an
        // already-known linear form must never count as hitting
        // `max_product_states`, only genuinely *new* ones may. `a*`'s
        // derivative w.r.t. 'a' is `a*` itself -- a self-loop, exactly one
        // distinct linear form for the whole walk -- so with
        // `max_product_states: 1`, matching a long run of 'a's against
        // `a*` must still succeed. Before the interning fix this wasn't
        // even the code path in question (the old premature-looking
        // `distinct.len() >= max_product_states` check was already only
        // evaluated once per character rather than once per *new* state,
        // which is exactly the bug class this test would have caught).
        let config = Config {
            max_product_states: 1,
            ..Config::default()
        };
        let report =
            analyze_match_with_backend("a*", &"a".repeat(50), &config, &AntimirovBackend).unwrap();
        assert_eq!(report.verdict, Verdict::Yes);
    }

    #[test]
    fn match_walk_cycling_between_two_known_states_survives_a_tight_state_limit() {
        // Same guard as above, but for a walk that cycles between *two*
        // distinct states rather than self-looping on one: `(ab)*` against
        // "ababab" alternates between the start state and the
        // mid-literal-b state six times, but only ever visits those same
        // two linear forms, so `max_product_states: 2` must be enough
        // regardless of the input's length.
        let config = Config {
            max_product_states: 2,
            ..Config::default()
        };
        let report =
            analyze_match_with_backend("(ab)*", "ababab", &config, &AntimirovBackend).unwrap();
        assert_eq!(report.verdict, Verdict::Yes);
    }

    #[test]
    fn match_new_state_still_respects_a_tight_state_limit() {
        // Complements the two tests above: confirms the limit still fires
        // when a walk genuinely needs more distinct states than budgeted,
        // so the interning fix hasn't accidentally made the limit
        // toothless. Matching "ab" against pattern `ab` needs exactly
        // three distinct linear forms in sequence -- {ab}, then {b}, then
        // {ε} -- so a budget of 2 must not stretch to the third.
        let config = Config {
            max_product_states: 2,
            ..Config::default()
        };
        let report = analyze_match_with_backend("ab", "ab", &config, &AntimirovBackend).unwrap();
        assert_eq!(report.verdict, Verdict::Unknown);
    }

    #[test]
    fn dedup_is_order_independent_for_same_length_chains() {
        // General regression guard for `LinearForm::from_parts`'s
        // sort+dedup (independent of any particular `reg_ord`
        // implementation): construction order must never affect the
        // normalized result, and duplicates must always collapse.
        //
        // Build the same set of terms two different ways -- forwards and
        // reversed -- and check `LinearForm::from_parts` produces the
        // identical, fully-deduplicated result regardless of input order.
        // The terms are deliberately different-length chains of the same
        // repeated element (the shape a `(a?)^n` derivative walk
        // produces), since that shape is what past attempts at optimizing
        // `reg_ord` for this codebase have touched -- this test is meant
        // to survive whichever comparator implementation is in place.
        fn chain(n: usize) -> Reg {
            let opt_a = Reg::alt(vec![Reg::eps(), Reg::atom(CharSet::singleton('a'))]);
            Reg::concat(vec![opt_a; n])
        }
        let forward: Vec<Reg> = (0..8).map(chain).collect();
        let mut backward = forward.clone();
        backward.reverse();
        // Duplicate a couple of entries too, so dedup has real work to do.
        let mut with_dupes = forward.clone();
        with_dupes.push(chain(3));
        with_dupes.push(chain(3));
        with_dupes.push(chain(0));

        let a = LinearForm::from_parts(forward);
        let b = LinearForm::from_parts(backward);
        let c = LinearForm::from_parts(with_dupes);
        assert_eq!(
            a, b,
            "order of construction must not affect the normalized form"
        );
        assert_eq!(
            a, c,
            "duplicate entries must still collapse to the same normalized form"
        );
        assert_eq!(
            a.0.len(),
            8,
            "8 distinct chain lengths, no accidental merging"
        );
    }

    #[test]
    fn match_bounded_optional_chain_no_wrapping_star() {
        // Correctness check on the actual shape of the
        // `match_500_optional_*` bench files: N concatenated `a?`s with no
        // outer `*`, i.e. a genuine bounded counter ("0 to N a's"), not
        // the `antimirov-block-position` family's collapsible-to-few-states
        // shape. Kept small (N=60) so it's a fast, deterministic
        // correctness check, independent of whatever the current
        // performance characteristics of the real N=500 case happen to
        // be (that's an open perf question -- see the `match_500_optional_*`
        // bench investigation notes -- this test only guards correctness).
        let pattern = "a?".repeat(60);
        let config = Config::default();
        let within =
            analyze_match_with_backend(&pattern, &"a".repeat(40), &config, &AntimirovBackend)
                .unwrap();
        assert_eq!(within.verdict, Verdict::Yes);
        let exact =
            analyze_match_with_backend(&pattern, &"a".repeat(60), &config, &AntimirovBackend)
                .unwrap();
        assert_eq!(exact.verdict, Verdict::Yes);
        let too_many =
            analyze_match_with_backend(&pattern, &"a".repeat(61), &config, &AntimirovBackend)
                .unwrap();
        assert_eq!(too_many.verdict, Verdict::No);
    }
}
