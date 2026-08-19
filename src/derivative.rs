//! Brzozowski-derivative backend for regular-language relation checking.
//!
//! Where [`AutomataBackend`] explores on-the-fly NFA subset products and
//! [`MinimizedBackend`] determinizes then minimizes, this backend derives
//! residual regular expressions symbolically and runs a product BFS over
//! pairs of normalized residuals. Nullable residuals decide acceptance;
//! alphabet partitions come from the character sets appearing in the current
//! residual pair, the same interval-boundary idea used elsewhere.
//!
//! Normalization keeps the residual DAG compact enough for the supported
//! subset: empty/identity elimination, alternation sorting and deduplication,
//! and flattened concatenations. Counted repetition from the AST is expanded
//! into concatenation / optional / star form before derivation begins.
//!
//! The binary product search also prunes product states once the query can
//! no longer be decided by anything reachable from them -- see
//! [`is_dead_end`] for the exact, query-specific, and query-asymmetric
//! condition and why it is sound.

use crate::analysis::{BackendResult, BackendStatus, Query, RelationBackend};
use crate::ast::{Expr, ExprKind};
use crate::charset::CharSet;
use crate::config::Config;
use crate::nfa::Nfa;
use crate::report::relation;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Normalized residual expression. Interned via [`Rc`] so product states can
/// share structure and cheaply clone keys for the visited set.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
enum RegKind {
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
/// shared subterm repeatedly (e.g. as a `HashMap<(Reg, char), Reg>` key
/// in `search_product`'s caches, or in `ResidualInterner`'s
/// `HashMap<Reg, usize>`) costs O(size) *every single time*, hit or miss.
fn compute_hash(kind: &RegKind) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    kind.hash(&mut hasher);
    hasher.finish()
}

#[derive(Clone, Debug)]
struct Reg(Rc<(RegKind, u64)>);

impl std::hash::Hash for Reg {
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

    fn alt(parts: Vec<Reg>) -> Self {
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
        let mut seen: HashSet<Reg> = HashSet::with_capacity(flat.len());
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

    fn nullable(&self) -> bool {
        match self.kind() {
            RegKind::Null | RegKind::Atom(_) => false,
            RegKind::Eps | RegKind::Star(_) => true,
            RegKind::Concat(parts) => parts.iter().all(Reg::nullable),
            RegKind::Alt(parts) => parts.iter().any(Reg::nullable),
        }
    }

    /// Collect outermost character sets that can start a non-empty word.
    fn first_sets(&self, out: &mut Vec<CharSet>) {
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

    /// Computes ∂ᶜʰ(self), memoized in `cache`.
    ///
    /// `cache` is expected to live for the whole search -- the same map
    /// `search_product` / `search_single` / the `match` walk already carry
    /// across their own top-level per-state lookups -- not just one call.
    /// That's what lets work done while unwinding one `Concat`'s
    /// derivative stay visible to every *other* state that later turns out
    /// to need one of the same intermediate suffixes: see the `Concat` arm
    /// below and its analogue, `partial_der`'s `Concat` case, in
    /// `antimirov.rs`.
    fn derivative(&self, ch: char, cache: &mut HashMap<(Reg, char), Reg>) -> Reg {
        let key = (self.clone(), ch);
        if let Some(cached) = cache.get(&key) {
            return cached.clone();
        }
        let result = match self.kind() {
            RegKind::Null | RegKind::Eps => Self::null(),
            RegKind::Atom(set) => {
                if set.contains(ch) {
                    Self::eps()
                } else {
                    Self::null()
                }
            }
            RegKind::Concat(parts) => {
                // ∂(e₁e₂…eₖ) = ∂(e₁)·(e₂…eₖ)  ∪  (if ν(e₁)) ∂(e₂…eₖ)
                //
                // Peel exactly one element and recurse into the tail
                // *through `cache`*, rather than looping over every split
                // point and rebuilding an independent O(len) suffix clone
                // for each one (the previous implementation). For a chain
                // of `k` nullable elements -- `a?` repeated, or any
                // `{m,n}`-expanded optional run, both with no wrapping
                // outer `*` -- that old loop cost O(k) per state and ran
                // once per state, with nothing shared between them: O(k²)
                // total for one state's full unwind, repeated
                // independently at each of the ~2k distinct states such a
                // chain produces, for O(k³) overall. Recursing through the
                // shared cache instead means the *first* state to reach
                // any given suffix computes and caches it once; every
                // later state that needs that same suffix (there are many,
                // since each is literally a shorter suffix of the one
                // before it) gets an O(1) hit. Confirmed empirically on a
                // flat (no wrapping `*`) chain of this shape --
                // `mega_equivalent__500-plus-vs-499-counted` went from
                // 7.3s to 0.9s, `mega_equivalent__250-optional` from
                // 13.5s to 4.3s -- bringing per-state cost within ~2% of
                // `AntimirovBackend`'s on the same states. A *star-wrapped*
                // chain of the same shape (`mega_equivalent__antimirov-
                // block-position-*`) sees a smaller win (still real, ~4x
                // fewer ms/state) but remains well outside the default
                // budget: each top-level state there is a
                // `Concat([Alt(..up to N members..), Star(..)])`, and
                // while each *member's* own derivative is cached by this
                // change, the surrounding `Alt`'s flatten/dedup/sort step
                // is reconstructed fresh per distinct wrapping state, an
                // O(N) cost this change doesn't amortize away. Left as a
                // separate, narrower follow-up rather than folded into
                // this fix.
                if parts.is_empty() {
                    Self::null()
                } else {
                    let head = &parts[0];
                    let tail = if parts.len() == 1 {
                        Self::eps()
                    } else {
                        Self::concat(parts[1..].to_vec())
                    };
                    let head_branch = Self::concat(vec![head.derivative(ch, cache), tail.clone()]);
                    if head.nullable() {
                        Self::alt(vec![head_branch, tail.derivative(ch, cache)])
                    } else {
                        head_branch
                    }
                }
            }
            RegKind::Alt(parts) => {
                Self::alt(parts.iter().map(|p| p.derivative(ch, cache)).collect())
            }
            RegKind::Star(inner) => {
                // ∂(r*) = ∂r · r*
                Self::concat(vec![inner.derivative(ch, cache), self.clone()])
            }
        };
        cache.insert(key, result.clone());
        result
    }
}

/// Deterministic total order for alternation sorting / dedup.
fn reg_ord(a: &Reg, b: &Reg) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    fn rank(r: &RegKind) -> u8 {
        match r {
            RegKind::Null => 0,
            RegKind::Eps => 1,
            RegKind::Atom(_) => 2,
            RegKind::Concat(_) => 3,
            RegKind::Alt(_) => 4,
            RegKind::Star(_) => 5,
        }
    }
    let ka = a.kind();
    let kb = b.kind();
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
            for (a, b) in ia.iter().zip(ib.iter()) {
                match (a.start, a.end).cmp(&(b.start, b.end)) {
                    Ordering::Equal => {}
                    other => return other,
                }
            }
            Ordering::Equal
        }
        (RegKind::Concat(pa), RegKind::Concat(pb)) => {
            // Length first, *then* elementwise -- not the reverse. Two
            // lists of different length can never be equal, so checking
            // length up front lets most comparisons between
            // differently-sized terms finish in O(1) instead of walking
            // min(len) shared elements only to fall back to the same
            // length check anyway.
            //
            // This matters far more than it looks like it should: for a
            // pattern like 500 concatenated `a?`s (no wrapping `*`), each
            // derivative step's residual is a single `Alt` of up to ~n
            // branches -- `Concat` chains that are literally suffixes of
            // the same repeated element, differing *only* in length.
            // Comparing two of them elementwise-first walks every shared
            // element (all trivially equal) before ever reaching the part
            // that actually distinguishes them -- turning what should be
            // an O(1) decision into an O(n) one, repeated across every
            // comparison in the sort. Confirmed on the analogous
            // `AntimirovBackend` case (same shape, same cost) before
            // porting the same fix here.
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
        (RegKind::Alt(pa), RegKind::Alt(pb)) => {
            // Same length-first rationale as the Concat case above.
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

/// Expand counted repetition into concat / optional / star form.
fn expand_repeat(inner: Reg, min: usize, max: Option<usize>) -> Reg {
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

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct ProductKey {
    left: Reg,
    right: Reg,
}

#[derive(Clone, Debug)]
struct SearchNode {
    key: ProductKey,
    parent: Option<(usize, char)>,
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

/// Whether a product state can be dropped from the frontier without ever
/// expanding it further.
///
/// `RegKind::Null` (`∅`) is an absorbing sink under [`Reg::derivative`]:
/// `∂(∅, c) = ∅` for every `c`, and `∅` never nullable. So once a residual
/// on the relevant side of a pair is `Null`, every state reachable from this
/// one has that same side permanently `Null` too, and `classify_relation`
/// can never fire again *from that side*. Skipping expansion here is a pure
/// search-space pruning: it changes which states get visited, never which
/// verdict is reachable.
///
/// The condition is deliberately query-specific and asymmetric -- getting
/// it wrong in the "more aggressive" direction produces an unsound early
/// `Exhausted`, not just a slower search:
///
/// - [`Query::Overlap`] needs `left_accepts && right_accepts`
///   simultaneously; once *either* side is `Null` that conjunction can never
///   hold again, so pruning on either side alone is sound.
/// - [`Query::Includes`] needs `left_accepts && !right_accepts`; once
///   `left` is `Null`, `left_accepts` is false forever, so the conjunction
///   can never hold regardless of what `right` does. The mirror case is
///   *not* safe to prune: `right` going `Null` makes `!right_accepts`
///   permanently true, which only helps satisfy the condition the next time
///   `left` accepts, so `left`'s side must keep being explored.
/// - [`Query::Equivalent`] fires on either `left_only` or `right_only`, so a
///   lone dead side still leaves the other side capable of firing the
///   opposite branch later. Pruning is only sound once *both* sides are
///   `Null` (see `equivalent_pruning_still_finds_a_right_only_witness`
///   below for a regression case that would fail under the too-eager
///   "either side" rule).
fn is_dead_end(query: Query, left: &Reg, right: &Reg) -> bool {
    let left_dead = matches!(left.kind(), RegKind::Null);
    let right_dead = matches!(right.kind(), RegKind::Null);
    match query {
        Query::Overlap => left_dead || right_dead,
        Query::Includes => left_dead,
        Query::Equivalent => left_dead && right_dead,
        // `is_dead_end` is only ever called from `search_product`, which
        // only runs for the three binary queries above; `Query::Empty`
        // takes the unary `search_single` path instead and never reaches
        // here. `false` (never prune) is the conservative, always-safe
        // answer for a query this function isn't actually asked about.
        Query::Empty | Query::Match => false,
    }
}

fn representative_chars(sets: &[CharSet]) -> Vec<char> {
    let mut boundaries = Vec::new();
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

fn reconstruct(nodes: &[SearchNode], mut node_id: usize) -> String {
    let mut reversed = Vec::new();
    while let Some((parent, ch)) = nodes[node_id].parent {
        reversed.push(ch);
        node_id = parent;
    }
    reversed.reverse();
    reversed.into_iter().collect()
}

fn stopped_result(
    status: BackendStatus,
    visited_states: usize,
    generated_transitions: usize,
    started: Instant,
) -> BackendResult {
    BackendResult {
        status,
        witness: None,
        relation: None,
        visited_states,
        generated_transitions,
        analysis_ms: started.elapsed().as_millis(),
        witness_extraction_ms: 0,
    }
}

fn search_product(query: Query, left: Reg, right: Reg, config: &Config) -> BackendResult {
    let started = Instant::now();
    let deadline = Duration::from_millis(config.timeout_ms);
    let initial = ProductKey { left, right };
    let mut nodes = vec![SearchNode {
        key: initial.clone(),
        parent: None,
    }];
    let mut visited = HashSet::new();
    visited.insert(initial);
    let mut queue = VecDeque::from([0usize]);
    let mut generated_transitions = 0usize;
    // Memoize derivatives so repeated residuals share work.
    let mut left_cache: HashMap<(Reg, char), Reg> = HashMap::new();
    let mut right_cache: HashMap<(Reg, char), Reg> = HashMap::new();

    while let Some(node_id) = queue.pop_front() {
        if started.elapsed() >= deadline {
            return stopped_result(
                BackendStatus::Timeout,
                nodes.len(),
                generated_transitions,
                started,
            );
        }

        let left_accepts = nodes[node_id].key.left.nullable();
        let right_accepts = nodes[node_id].key.right.nullable();
        if let Some(rel) = classify_relation(query, left_accepts, right_accepts) {
            let analysis_ms = started.elapsed().as_millis();
            let witness_started = Instant::now();
            let witness = reconstruct(&nodes, node_id);
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
            // Every state reachable from here can never satisfy this
            // query's classification either (see `is_dead_end`'s doc
            // comment); stop at this node instead of expanding it.
            continue;
        }

        let mut first = Vec::new();
        nodes[node_id].key.left.first_sets(&mut first);
        nodes[node_id].key.right.first_sets(&mut first);
        let reps = representative_chars(&first);

        for ch in reps {
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
            let next_left = nodes[node_id].key.left.derivative(ch, &mut left_cache);
            let next_right = nodes[node_id].key.right.derivative(ch, &mut right_cache);
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
    struct Key(Reg);
    struct Node {
        key: Key,
        parent: Option<(usize, char)>,
    }

    let initial = Key(reg);
    let mut nodes = vec![Node {
        key: initial.clone(),
        parent: None,
    }];
    let mut visited = HashSet::new();
    visited.insert(initial);
    let mut queue = VecDeque::from([0usize]);
    let mut generated_transitions = 0usize;
    let mut cache: HashMap<(Reg, char), Reg> = HashMap::new();

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
            let next_reg = nodes[node_id].key.0.derivative(ch, &mut cache);
            let next = Key(next_reg);
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

/// Brzozowski-derivative [`RelationBackend`].
///
/// Overrides the AST-facing entry points; the NFA-only trait methods are
/// implemented as a fallback that would only run if a caller bypassed the
/// expression path (they are not used by the normal analysis pipeline).
#[derive(Clone, Copy, Debug, Default)]
pub struct DerivativeBackend;

impl RelationBackend for DerivativeBackend {
    fn name(&self) -> &'static str {
        "brzozowski_derivatives"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn analyze_binary(
        &self,
        _query: Query,
        _left: &Nfa,
        _right: &Nfa,
        _config: &Config,
    ) -> BackendResult {
        // Prefer analyze_binary_expr; this path has no AST and cannot derive.
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
        match_brzozowski(expr, input, config)
    }
}

/// Walk one concrete string under Brzozowski derivation with memoization.
///
/// A tiny lazily-built DFA over `Reg` residual states, keyed by small
/// integer ids rather than by the residuals themselves.
///
/// `match_brzozowski` caches transitions as `HashMap<(usize, char),
/// usize>`, not `HashMap<(Reg, char), Reg>`: the latter would still need
/// to *hash* the current `Reg` on every lookup to find it, hit or miss.
/// `Reg` is hash-consed (its hash is computed once, at construction, and
/// cached -- see `compute_hash`), so that hashing is O(1), but it's still
/// avoided here entirely in favor of comparing small integers, which is
/// cheaper still and needs no hashing at all.
///
/// Interning means a revisited residual is a `(usize, char) -> usize`
/// lookup, cheap regardless of how large the underlying residual is. This
/// is the same "lazy DFA" technique real derivative-based regex engines
/// use to make repeated/long matching fast without a separate upfront
/// determinization pass.
struct ResidualInterner {
    ids: HashMap<Reg, usize>,
    states: Vec<Reg>,
}

impl ResidualInterner {
    fn new(start: Reg) -> Self {
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
    fn intern(&mut self, state: Reg, max_states: usize) -> Option<usize> {
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

/// Only residuals actually visited by `input` are computed — the classic
/// derivative advantage over building a full automaton for membership.
fn match_brzozowski(expr: &Expr, input: &str, config: &Config) -> BackendResult {
    let started = Instant::now();
    let deadline = Duration::from_millis(config.timeout_ms);
    let mut interner = ResidualInterner::new(from_expr(expr));
    let mut transitions: HashMap<(usize, char), usize> = HashMap::new();
    // Spans the whole walk, like `search_product`'s/`search_single`'s own
    // caches -- previously this call site had no cache at all, so each new
    // (state, char) transition's derivative was computed from a cold
    // start even when it shared large suffixes with transitions already
    // computed earlier in the same walk.
    let mut cache: HashMap<(Reg, char), Reg> = HashMap::new();
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
                let next_reg = interner.states[current].derivative(ch, &mut cache);
                match interner.intern(next_reg, config.max_product_states) {
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
        // Empty residual: no continuation can accept.
        if matches!(interner.states[current].kind(), RegKind::Null) {
            break;
        }
    }

    let analysis_ms = started.elapsed().as_millis();
    let reg = &interner.states[current];
    if reg.nullable() {
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

    fn both_agree(query: Query, left: &str, right: &str) {
        let config = Config::default();
        let a = analyze_binary_with_backend(query, left, right, &config, &AutomataBackend).unwrap();
        let b =
            analyze_binary_with_backend(query, left, right, &config, &DerivativeBackend).unwrap();
        assert_eq!(
            a.verdict, b.verdict,
            "backends disagree on {:?}({:?}, {:?}): automata={:?} derivatives={:?}",
            query, left, right, a.verdict, b.verdict
        );
        assert_eq!(
            a.witness.map(|w| w.value),
            b.witness.map(|w| w.value),
            "backends produced different witnesses for {:?}({:?}, {:?})",
            query,
            left,
            right
        );
    }

    #[test]
    fn nullable_basics() {
        let config = Config::default();
        assert!(from_expr(&parse("", &config).unwrap()).nullable());
        assert!(!from_expr(&parse("a", &config).unwrap()).nullable());
        assert!(from_expr(&parse("a*", &config).unwrap()).nullable());
        assert!(!from_expr(&parse("a+", &config).unwrap()).nullable());
        assert!(from_expr(&parse("a?", &config).unwrap()).nullable());
    }

    #[test]
    fn empty_alternation_denotes_the_empty_language() {
        // `parse` can never build `ExprKind::Alt(vec![])` (see the matching
        // test and comment in `nfa.rs`), so this exercises `from_expr`
        // directly against the AST node a library caller could still
        // construct by hand. An empty alternation is the identity element
        // for union and must normalize to `RegKind::Null` (`∅`) -- exactly
        // what `Reg::alt` already does for an empty branch list once
        // flattening removes everything, so `from_expr` just needs to reach
        // that same path instead of special-casing `{ε}`.
        use crate::ast::Span;
        let expr = Expr::new(ExprKind::Alt(Vec::new()), Span::new(0, 0));
        assert_eq!(from_expr(&expr), Reg::null());
        assert!(!from_expr(&expr).nullable());
    }

    #[test]
    fn derivative_of_literal() {
        let config = Config::default();
        let r = from_expr(&parse("a", &config).unwrap());
        let mut cache = HashMap::new();
        assert_eq!(r.derivative('a', &mut cache), Reg::eps());
        assert_eq!(r.derivative('b', &mut cache), Reg::null());
    }

    #[test]
    fn agrees_with_automata_on_corpus() {
        let cases: &[(&str, &str)] = &[
            ("a+b", "ab+"),
            ("a+", "b+"),
            ("[a-z]+", "[a-z]{2,}"),
            ("a|b", "[ab]"),
            ("b", "a|b"),
            ("(a*b*)*", "(a|b)*"),
            ("(a?)*", "a*"),
            ("a{2,3}", "aa(a?)"),
            ("[^a-c]", "[^b-d]"),
            ("\\d+", "\\w+"),
            ("\\w+", "\\d+"),
            ("[a-c]+", "[c-e]+"),
            ("a*", "a*a*"),
            ("(a|b)(a|b)*", "(a|b)+"),
            ("", "a*"),
            ("a{0}", ""),
        ];
        for &(left, right) in cases {
            for query in [Query::Overlap, Query::Includes, Query::Equivalent] {
                both_agree(query, left, right);
            }
        }
    }

    #[test]
    fn agrees_on_emptiness() {
        let config = Config::default();
        for pattern in ["a*", "a+", "a{0}", r"[^\d\D]", ""] {
            let a = analyze_empty_with_backend(pattern, &config, &AutomataBackend).unwrap();
            let b = analyze_empty_with_backend(pattern, &config, &DerivativeBackend).unwrap();
            assert_eq!(
                a.verdict, b.verdict,
                "backends disagree on empty({:?})",
                pattern
            );
            assert_eq!(
                a.witness.map(|w| w.value),
                b.witness.map(|w| w.value),
                "backends produced different witnesses for empty({:?})",
                pattern
            );
        }
    }

    #[test]
    fn overlap_demo_case() {
        let report = analyze_binary_with_backend(
            Query::Overlap,
            "a+b",
            "ab+",
            &Config::default(),
            &DerivativeBackend,
        )
        .unwrap();
        assert_eq!(report.verdict, Verdict::Yes);
        assert_eq!(report.witness.unwrap().value, "ab");
        assert_eq!(report.backend.name, "brzozowski_derivatives");
    }

    #[test]
    fn overlap_pruning_makes_search_independent_of_bounded_repetition_width() {
        // Without dead-branch pruning, disproving overlap between a bounded
        // repetition and a disjoint single character has to walk every
        // reachable state of the repetition, even though the right-hand
        // side dies (becomes `Null`, via `is_dead_end`) after the very
        // first character and can never contribute an overlap witness
        // again. With pruning in place, the visited-state count no longer
        // grows with the repetition's width.
        let config = Config::default();
        let small =
            analyze_binary_with_backend(Query::Overlap, "a{0,5}", "b", &config, &DerivativeBackend)
                .unwrap();
        let large = analyze_binary_with_backend(
            Query::Overlap,
            "a{0,200}",
            "b",
            &config,
            &DerivativeBackend,
        )
        .unwrap();
        assert_eq!(small.verdict, Verdict::No);
        assert_eq!(large.verdict, Verdict::No);
        assert_eq!(
            small.statistics.visited_product_states, large.statistics.visited_product_states,
            "visited-state count should not grow with the repetition width \
             once the permanently-dead right-hand side is pruned"
        );
    }

    #[test]
    fn includes_pruning_makes_search_independent_of_right_hand_width() {
        // Mirror of the overlap case above, but for `includes`: the
        // left-hand side dies after one character while the right-hand
        // side (a much wider bounded repetition that always accepts) keeps
        // going. `is_dead_end` only prunes `includes` once `left` dies, so
        // this also exercises that the asymmetric rule fires correctly.
        let config = Config::default();
        let small = analyze_binary_with_backend(
            Query::Includes,
            "a{0,1}",
            "[abcd]{0,5}",
            &config,
            &DerivativeBackend,
        )
        .unwrap();
        let large = analyze_binary_with_backend(
            Query::Includes,
            "a{0,1}",
            "[abcd]{0,200}",
            &config,
            &DerivativeBackend,
        )
        .unwrap();
        assert_eq!(small.verdict, Verdict::Yes);
        assert_eq!(large.verdict, Verdict::Yes);
        assert_eq!(
            small.statistics.visited_product_states, large.statistics.visited_product_states,
            "visited-state count should not grow with the right-hand width \
             once the permanently-dead left-hand side is pruned"
        );
    }

    #[test]
    fn equivalent_pruning_still_finds_a_right_only_witness() {
        // Regression guard for `is_dead_end`'s query-specific asymmetry:
        // for `equivalent`, pruning is only sound once *both* sides are
        // `Null`. Here the left side (`a{0,2}`) dies well before the right
        // side, which still accepts "aaaaa" -- an overly eager "prune once
        // either side is dead" rule (the wrong generalization from the
        // `overlap` case) would stop the search before ever reaching that
        // state and would wrongly report the two patterns as equivalent.
        let report = analyze_binary_with_backend(
            Query::Equivalent,
            "a{0,2}",
            "a{0,2}|a{5}",
            &Config::default(),
            &DerivativeBackend,
        )
        .unwrap();
        assert_eq!(report.verdict, Verdict::No);
        assert_eq!(report.witness.unwrap().value, "aaaaa");
    }

    #[test]
    fn duplicated_alternation_branches_collapse_faster_than_raw_nfa_subsets() {
        // A case the derivative backend already handles better than
        // `AutomataBackend` with no engine change: normalization
        // (`Reg::alt`'s flatten + sort + dedup) collapses exact-duplicate
        // branches the moment the residual is built, while
        // `AutomataBackend`'s on-the-fly subset construction has to
        // discover the same fact by exploring actual NFA-state-ID subsets,
        // since Thompson construction gives every branch (duplicate or not)
        // its own private states. This models a realistic case for this
        // tool's stated audience (SPEC.md: "route, identifier, filename, or
        // policy patterns"): an allowlist regex that accidentally lists
        // some entries twice, checked for equivalence against its
        // deduplicated form.
        let words = [
            "users",
            "orders",
            "payments",
            "invoices",
            "accounts",
            "sessions",
            "tokens",
            "webhooks",
            "products",
            "carts",
            "refunds",
            "shipments",
        ];
        let deduped = words.join("|");
        let with_duplicates = format!("{deduped}|{deduped}");

        let config = Config::default();
        let automata = analyze_binary_with_backend(
            Query::Equivalent,
            &with_duplicates,
            &deduped,
            &config,
            &AutomataBackend,
        )
        .unwrap();
        let derivatives = analyze_binary_with_backend(
            Query::Equivalent,
            &with_duplicates,
            &deduped,
            &config,
            &DerivativeBackend,
        )
        .unwrap();

        assert_eq!(automata.verdict, Verdict::Yes);
        assert_eq!(derivatives.verdict, Verdict::Yes);
        assert!(
            derivatives.statistics.visited_product_states
                < automata.statistics.visited_product_states,
            "expected derivatives to visit fewer product states than automata \
             (derivatives={}, automata={})",
            derivatives.statistics.visited_product_states,
            automata.statistics.visited_product_states,
        );
    }

    #[test]
    fn respects_state_limit() {
        let config = Config {
            max_product_states: 1,
            ..Config::default()
        };
        // Non-empty start: empty string is not a counterexample for a vs b
        // equivalence, so the search must expand and hit the limit.
        let report =
            analyze_binary_with_backend(Query::Equivalent, "a+", "b+", &config, &DerivativeBackend)
                .unwrap();
        assert_eq!(report.verdict, Verdict::Unknown);
    }

    #[test]
    fn match_walk_revisiting_a_known_state_survives_a_tight_state_limit() {
        // Regression guard for `ResidualInterner::intern`: revisiting an
        // already-known residual must never count as hitting
        // `max_product_states`, only genuinely *new* ones may. `a*`'s
        // derivative w.r.t. 'a' is `a*` itself -- a self-loop, exactly one
        // distinct residual for the whole walk -- so with
        // `max_product_states: 1`, matching a long run of 'a's against
        // `a*` must still succeed.
        let config = Config {
            max_product_states: 1,
            ..Config::default()
        };
        let report =
            analyze_match_with_backend("a*", &"a".repeat(50), &config, &DerivativeBackend).unwrap();
        assert_eq!(report.verdict, Verdict::Yes);
    }

    #[test]
    fn match_walk_cycling_between_two_known_states_survives_a_tight_state_limit() {
        // Same guard as above, but for a walk that cycles between *two*
        // distinct residuals rather than self-looping on one: `(ab)*`
        // against "ababab" alternates between the start residual and the
        // mid-literal-b residual six times, but only ever visits those
        // same two residuals, so `max_product_states: 2` must be enough
        // regardless of the input's length.
        let config = Config {
            max_product_states: 2,
            ..Config::default()
        };
        let report =
            analyze_match_with_backend("(ab)*", "ababab", &config, &DerivativeBackend).unwrap();
        assert_eq!(report.verdict, Verdict::Yes);
    }

    #[test]
    fn match_new_state_still_respects_a_tight_state_limit() {
        // Complements the two tests above: confirms the limit still fires
        // when a walk genuinely needs more distinct residuals than
        // budgeted, so the interning fix hasn't accidentally made the
        // limit toothless. Matching "ab" against pattern `ab` needs
        // exactly three distinct residuals in sequence -- `ab`, then `b`,
        // then `ε` -- so a budget of 2 must not stretch to the third.
        let config = Config {
            max_product_states: 2,
            ..Config::default()
        };
        let report = analyze_match_with_backend("ab", "ab", &config, &DerivativeBackend).unwrap();
        assert_eq!(report.verdict, Verdict::Unknown);
    }

    #[test]
    fn dedup_is_order_independent_for_same_length_chains() {
        // General regression guard for `Reg::alt`'s sort+dedup
        // (independent of any particular `reg_ord` implementation):
        // construction order must never affect the normalized result, and
        // duplicates must always collapse.
        //
        // Build the same set of terms two different ways -- forwards and
        // reversed -- and check `Reg::alt` produces the identical, fully
        // deduplicated result regardless of input order. The terms are
        // deliberately different-length chains of the same repeated
        // element (the shape a `(a?)^n` derivative walk produces), since
        // that shape is what past attempts at optimizing `reg_ord` for
        // this codebase have touched -- this test is meant to survive
        // whichever comparator implementation is in place.
        fn chain(n: usize) -> Reg {
            let opt_a = Reg::alt(vec![Reg::eps(), Reg::atom(CharSet::singleton('a'))]);
            Reg::concat(vec![opt_a; n])
        }
        let forward: Vec<Reg> = (0..8).map(chain).collect();
        let mut backward = forward.clone();
        backward.reverse();
        let mut with_dupes = forward.clone();
        with_dupes.push(chain(3));
        with_dupes.push(chain(3));
        with_dupes.push(chain(0));

        let a = Reg::alt(forward);
        let b = Reg::alt(backward);
        let c = Reg::alt(with_dupes);
        assert_eq!(
            a, b,
            "order of construction must not affect the normalized form"
        );
        assert_eq!(
            a, c,
            "duplicate entries must still collapse to the same normalized form"
        );
        match a.kind() {
            RegKind::Alt(parts) => assert_eq!(parts.len(), 8, "8 distinct chain lengths"),
            other => panic!("expected a flattened Alt of 8 distinct terms, got {other:?}"),
        }
    }

    #[test]
    fn match_bounded_optional_chain_no_wrapping_star() {
        // Correctness check on the actual shape of the
        // `match_500_optional_*` bench files: N concatenated `a?`s with no
        // outer `*`, i.e. a genuine bounded counter ("0 to N a's"), not a
        // shape that collapses to few states. Kept small (N=60) so it's a
        // fast, deterministic correctness check, independent of whatever
        // the current performance characteristics of the real N=500 case
        // happen to be (that's an open perf question -- see the
        // `match_500_optional_*` bench investigation notes -- this test
        // only guards correctness).
        let pattern = "a?".repeat(60);
        let config = Config::default();
        let within =
            analyze_match_with_backend(&pattern, &"a".repeat(40), &config, &DerivativeBackend)
                .unwrap();
        assert_eq!(within.verdict, Verdict::Yes);
        let exact =
            analyze_match_with_backend(&pattern, &"a".repeat(60), &config, &DerivativeBackend)
                .unwrap();
        assert_eq!(exact.verdict, Verdict::Yes);
        let too_many =
            analyze_match_with_backend(&pattern, &"a".repeat(61), &config, &DerivativeBackend)
                .unwrap();
        assert_eq!(too_many.verdict, Verdict::No);
    }
}
