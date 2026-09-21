//! Bisimulation up to congruence (Bonchi & Pous, POPL'13).
//!
//! Sixth backend, fifth independent [`RelationBackend`] (after `automata`,
//! `minimized`, `derivatives`, `antimirov`, `antichain`). Decides
//! `equivalent` directly, without ever determinizing either input: `A` and
//! `B` are combined into one NFA over a shared, disjoint state space (`A`'s
//! own states, then `B`'s, offset), and the search tracks a pair `(X, Y)`
//! of *subsets* of that combined space -- `X` always a subset of `A`'s
//! states, `Y` always a subset of `B`'s, since the two halves share no
//! transitions. `L(A) = L(B)` iff the initial pair `(IA, IB)` is bisimilar
//! under `o(S) = S ∩ final ≠ ∅` and `post`.
//!
//! The "up to congruence" part is what the theory says should make this
//! fast on patterns `minimized` chokes on: a discovered pair `(X, Y)`
//! never needs to be explored if it's already implied by *unions* of
//! previously-established pairs (`(U, V)` and `(U', V')` bisimilar implies
//! `(U∪U', V∪V')` is too). `congruent` below decides this via saturation
//! (`closure`), exactly as sketched in the paper: grow `X` by repeatedly
//! pulling in `V` whenever some established `(U, V)` has `U` already
//! inside the growing set (and symmetrically), then compare the two
//! saturated sets for equality. This is the *naive* O(|R|) - per -
//! saturation - pass version; the paper's incremental version (inverted
//! index + pending-count queue) is a planned follow-up once this is proven
//! correct -- see `docs/backends.md`. Whether pruning actually pays off in
//! practice, with this naive version, turns out to depend heavily on the
//! input -- see below before assuming this beats `minimized`.
//!
//! Query mapping:
//! - `equivalent` -- direct: `(IA, IB)`.
//! - `includes` -- `L(left) ⊆ L(right)` iff `L(left) ∪ L(right) = L(right)`,
//!   checked as `(IA ∪ IB, IB)` in the *same* combined space -- no separate
//!   union-NFA construction needed, since `IB ⊆ IA ∪ IB` always (`post` is
//!   monotone), so the only possible disagreement is "found in `IA ∪ IB`,
//!   not in `IB`", which is exactly a `left_only` witness.
//! - `empty` -- delegated to [`search_single`], the same NFA-reachability
//!   check `automata` and `antichain` already share; full bisimulation
//!   machinery is unwarranted overkill for plain reachability.
//! - `overlap` -- delegated entirely to [`AutomataBackend`]'s NFA×NFA
//!   product search. HKC brings no advantage here (overlap needs no
//!   subsets on either side to begin with, so there's nothing to
//!   determinize away), and this backend is not an independent decision
//!   procedure for `overlap` as a result -- differential testing against
//!   `automata` on `overlap` specifically has no value, since a disagreement
//!   there is definitionally impossible.
//!
//! **Performance, measured, not assumed:** the literature's headline
//! numbers for this technique are real, but they're for inputs with
//! genuine *union-decomposable* redundancy -- a newly reached pair
//! provably equal to a union of already-confirmed ones. Testing against
//! this crate's `minimized` and `automata` backends across generated
//! patterns, a few deliberately redundant patterns (repeated identical
//! alternation branches, nested redundant `?`), and this crate's own
//! suffix/window hard-suite family (`(a|b)*a(a|b){n}`) did not reproduce a
//! clear win in `visited_product_states` or wall time for any of them --
//! and on the window family specifically, `hkc` is consistently *worse*
//! than `minimized` (and gets worse, relatively, as `n` grows), because
//! that family needs genuinely non-redundant state tracking (one
//! independent bit per recent position) that union-closure cannot
//! collapse, so every one of these `congruent` calls does real work for
//! zero pruning payoff. Naive `closure` is also O(|R|) per call and the
//! loop calls it once per popped node, so cost compounds as `R` grows
//! regardless of whether pruning fires -- the incremental version (an
//! inverted index plus a pending-count queue, sketched in the design
//! notes this module was built from) is the documented fix for that, and
//! is not yet implemented. Until then, treat `hkc` as a genuinely
//! independent cross-check for `equivalent`/`includes`, not a faster
//! default than `minimized`/`antichain` -- see `docs/backends.md`.

use crate::analysis::{
    search_single, AutomataBackend, BackendResult, BackendStatus, Query, RelationBackend,
};
use crate::charset::representative_chars;
use crate::config::Config;
use crate::nfa::Nfa;
use crate::report::relation;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Packed bit-set over the combined `A ⊔ B` state space. Independent copy
/// of `antichain.rs`'s type (plus `intersects`, needed for the output
/// test) -- kept separate rather than shared so this module has no
/// compile-time dependency on `antichain`'s internals.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct BitSet {
    words: Vec<u64>,
}

impl BitSet {
    fn new(n: usize) -> Self {
        let n_words = if n == 0 { 0 } else { n.div_ceil(64) };
        Self {
            words: vec![0u64; n_words],
        }
    }

    fn from_ids(n: usize, ids: &[usize]) -> Self {
        let mut s = Self::new(n);
        for &id in ids {
            s.insert(id);
        }
        s
    }

    fn insert(&mut self, id: usize) {
        if self.words.is_empty() {
            return;
        }
        self.words[id / 64] |= 1u64 << (id % 64);
    }

    /// `self ⊆ other`
    fn is_subset(&self, other: &Self) -> bool {
        self.words
            .iter()
            .zip(other.words.iter())
            .all(|(a, b)| (a & !b) == 0)
    }

    fn intersects(&self, other: &Self) -> bool {
        self.words
            .iter()
            .zip(other.words.iter())
            .any(|(a, b)| (a & b) != 0)
    }

    fn union_with(&mut self, other: &Self) {
        for (a, b) in self.words.iter_mut().zip(other.words.iter()) {
            *a |= *b;
        }
    }

    fn union(&self, other: &Self) -> Self {
        let mut out = self.clone();
        out.union_with(other);
        out
    }

    fn iter(&self) -> BitIter {
        BitIter {
            words: self.words.clone(),
            word_idx: 0,
            current: self.words.first().copied().unwrap_or(0),
            started: false,
        }
    }
}

struct BitIter {
    words: Vec<u64>,
    word_idx: usize,
    current: u64,
    started: bool,
}

impl Iterator for BitIter {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        if !self.started {
            self.started = true;
        }
        loop {
            if self.current != 0 {
                let bit = self.current.trailing_zeros() as usize;
                self.current &= self.current - 1;
                return Some(self.word_idx * 64 + bit);
            }
            self.word_idx += 1;
            if self.word_idx >= self.words.len() {
                return None;
            }
            self.current = self.words[self.word_idx];
        }
    }
}

/// `A ⊔ B` in one shared address space: `A`'s states keep their own ids
/// (`0..n_a`), `B`'s are offset by `n_a`. Built once per query; `post`
/// (with caching) and `accepts` are the only operations the search loop
/// needs.
struct Combined {
    n: usize,
    ia: BitSet,
    ib: BitSet,
    final_mask: BitSet,
    n_parts: usize,
    delta: Vec<BitSet>,
}

impl Combined {
    fn new(a: &Nfa, b: &Nfa, parts: &[char]) -> Self {
        let n_a = a.states.len();
        let n_b = b.states.len();
        let n = n_a + n_b;
        let n_parts = parts.len();

        let mut eps: Vec<BitSet> = Vec::with_capacity(n);
        for q in 0..n_a {
            let closed = a.epsilon_closure(std::iter::once(q));
            eps.push(BitSet::from_ids(n, &closed));
        }
        for q in 0..n_b {
            let closed = b.epsilon_closure(std::iter::once(q));
            let offset: Vec<usize> = closed.into_iter().map(|s| s + n_a).collect();
            eps.push(BitSet::from_ids(n, &offset));
        }

        let mut delta = vec![BitSet::new(n); n * n_parts.max(1)];
        if n_parts > 0 {
            for q in 0..n_a {
                for (p, &ch) in parts.iter().enumerate() {
                    let mut post = BitSet::new(n);
                    for tr in &a.states[q].transitions {
                        if tr.set.contains(ch) {
                            post.union_with(&eps[tr.target]);
                        }
                    }
                    delta[q * n_parts + p] = post;
                }
            }
            for q in 0..n_b {
                for (p, &ch) in parts.iter().enumerate() {
                    let mut post = BitSet::new(n);
                    for tr in &b.states[q].transitions {
                        if tr.set.contains(ch) {
                            post.union_with(&eps[tr.target + n_a]);
                        }
                    }
                    delta[(q + n_a) * n_parts + p] = post;
                }
            }
        }

        let ia = eps[a.start].clone();
        let ib = eps[b.start + n_a].clone();

        let mut final_mask = BitSet::new(n);
        final_mask.insert(a.accept);
        final_mask.insert(b.accept + n_a);

        Self {
            n,
            ia,
            ib,
            final_mask,
            n_parts,
            delta,
        }
    }

    fn delta(&self, q: usize, p: usize) -> &BitSet {
        &self.delta[q * self.n_parts + p]
    }

    fn post(&self, s: &BitSet, p: usize, cache: &mut HashMap<(BitSet, usize), BitSet>) -> BitSet {
        if let Some(hit) = cache.get(&(s.clone(), p)) {
            return hit.clone();
        }
        let mut res = BitSet::new(self.n);
        for q in s.iter() {
            res.union_with(self.delta(q, p));
        }
        cache.insert((s.clone(), p), res.clone());
        res
    }

    fn accepts(&self, s: &BitSet) -> bool {
        s.intersects(&self.final_mask)
    }
}

fn global_parts(a: &Nfa, b: &Nfa) -> Vec<char> {
    let mut sets = Vec::new();
    for nfa in [a, b] {
        for state in &nfa.states {
            for tr in &state.transitions {
                sets.push(&tr.set);
            }
        }
    }
    representative_chars(&sets)
}

/// Saturate `s` under `r`'s pairs: whenever some `(u, v) ∈ r` has `u`
/// already inside `s`, `v` is pulled in too (and symmetrically). Naive
/// O(|r|) - per - pass fixed point; see the module doc for the planned
/// incremental replacement.
fn closure(r: &[(BitSet, BitSet)], mut s: BitSet) -> BitSet {
    loop {
        let mut changed = false;
        for (u, v) in r {
            if u.is_subset(&s) && !v.is_subset(&s) {
                s.union_with(v);
                changed = true;
            }
            if v.is_subset(&s) && !u.is_subset(&s) {
                s.union_with(u);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    s
}

/// `(x, y) ∈ c(r)` iff their saturations under `r` coincide (Bonchi & Pous,
/// Thm. -- see module doc).
fn congruent(r: &[(BitSet, BitSet)], x: &BitSet, y: &BitSet) -> bool {
    closure(r, x.clone()) == closure(r, y.clone())
}

struct Node {
    x: BitSet,
    y: BitSet,
    parent: Option<usize>,
    via: Option<char>,
}

fn reconstruct(arena: &[Node], mut id: usize) -> String {
    let mut chars = Vec::new();
    loop {
        let node = &arena[id];
        if let Some(ch) = node.via {
            chars.push(ch);
        }
        match node.parent {
            Some(p) => id = p,
            None => break,
        }
    }
    chars.reverse();
    chars.into_iter().collect()
}

fn found(
    witness: String,
    relation_id: &'static str,
    visited: usize,
    generated: usize,
    started: Instant,
) -> BackendResult {
    BackendResult {
        status: BackendStatus::Found,
        witness: Some(witness),
        relation: Some(relation_id.to_owned()),
        visited_states: visited,
        generated_transitions: generated,
        analysis_ms: started.elapsed().as_millis(),
        witness_extraction_ms: 0,
    }
}

fn stopped(
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

/// The shared search: bisimulation up to congruence between an initial
/// pair `(x0, y0)` in `combined`'s space. `Exhausted` (no witness) means
/// bisimilar -- i.e. `Verdict::Yes` once the caller maps status to verdict,
/// same convention as every other backend in this crate.
///
/// A single array doubles as both the node arena and the BFS queue (a
/// `head` index into it rather than a separate `VecDeque`): each popped
/// node pushes all of its children, in ascending character order, before
/// the next node is popped. That is what keeps this a correct shortest-
/// then-lowest-scalar search with no extra bookkeeping -- unlike
/// `antichain.rs`'s per-query BFS loops (see its `Group` type and the
/// history in `CHANGELOG.md`), there is no risk here of two states that
/// share a prefix ending up processed non-contiguously, because `X` and
/// `Y` are always full, already-closed subsets rather than individually
/// tracked states: there is exactly one child per (parent, character),
/// never several needing to be grouped.
fn check(
    combined: &Combined,
    parts: &[char],
    x0: BitSet,
    y0: BitSet,
    config: &Config,
) -> BackendResult {
    let started = Instant::now();
    let deadline = Duration::from_millis(config.timeout_ms);

    let mut r: Vec<(BitSet, BitSet)> = Vec::new();
    let mut cache: HashMap<(BitSet, usize), BitSet> = HashMap::new();
    let mut arena: Vec<Node> = vec![Node {
        x: x0,
        y: y0,
        parent: None,
        via: None,
    }];
    let mut head = 0usize;
    let mut generated = 0usize;

    while head < arena.len() {
        if started.elapsed() >= deadline {
            return stopped(BackendStatus::Timeout, arena.len(), generated, started);
        }
        if arena.len() > config.max_product_states {
            return stopped(BackendStatus::StateLimit, arena.len(), generated, started);
        }

        let idx = head;
        head += 1;

        if congruent(&r, &arena[idx].x, &arena[idx].y) {
            continue;
        }

        let xa = combined.accepts(&arena[idx].x);
        let yb = combined.accepts(&arena[idx].y);
        if xa != yb {
            let witness = reconstruct(&arena, idx);
            let relation_id = if xa {
                relation::LEFT_ONLY
            } else {
                relation::RIGHT_ONLY
            };
            return found(witness, relation_id, arena.len(), generated, started);
        }

        r.push((arena[idx].x.clone(), arena[idx].y.clone()));

        for (p, &ch) in parts.iter().enumerate() {
            generated += 1;
            let x1 = combined.post(&arena[idx].x, p, &mut cache);
            let y1 = combined.post(&arena[idx].y, p, &mut cache);
            arena.push(Node {
                x: x1,
                y: y1,
                parent: Some(idx),
                via: Some(ch),
            });
        }
    }

    stopped(BackendStatus::Exhausted, arena.len(), generated, started)
}

fn run_binary(query: Query, left: &Nfa, right: &Nfa, config: &Config) -> BackendResult {
    match query {
        Query::Overlap => AutomataBackend.analyze_binary(Query::Overlap, left, right, config),
        Query::Equivalent => {
            let parts = global_parts(left, right);
            let combined = Combined::new(left, right, &parts);
            let (x0, y0) = (combined.ia.clone(), combined.ib.clone());
            check(&combined, &parts, x0, y0, config)
        }
        Query::Includes => {
            let parts = global_parts(left, right);
            let combined = Combined::new(left, right, &parts);
            let x0 = combined.ia.union(&combined.ib);
            let y0 = combined.ib.clone();
            check(&combined, &parts, x0, y0, config)
        }
        Query::Empty | Query::Match => unreachable!("binary entry with a unary query"),
    }
}

/// Bisimulation-up-to-congruence engine (Bonchi & Pous, POPL'13).
#[derive(Clone, Copy, Debug, Default)]
pub struct HkcBackend;

impl RelationBackend for HkcBackend {
    fn name(&self) -> &'static str {
        "hkc"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn analyze_binary(
        &self,
        query: Query,
        left: &Nfa,
        right: &Nfa,
        config: &Config,
    ) -> BackendResult {
        run_binary(query, left, right, config)
    }

    fn analyze_empty(&self, nfa: &Nfa, config: &Config) -> BackendResult {
        search_single(nfa, config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::analyze_binary_with_backend;
    use crate::report::{Report, Verdict};

    fn check(query: Query, left: &str, right: &str) -> Report {
        let config = Config::default();
        analyze_binary_with_backend(query, left, right, &config, &HkcBackend).unwrap()
    }

    #[test]
    fn equivalent_yes() {
        for (l, r) in [("a+", "aa*"), ("(a|b)*", "(b|a)*"), ("a?a?a", "a{1,3}")] {
            let report = check(Query::Equivalent, l, r);
            assert_eq!(report.verdict, Verdict::Yes, "{l} vs {r}");
        }
    }

    #[test]
    fn equivalent_no_has_left_only_witness() {
        let report = check(Query::Equivalent, "a|b", "b");
        assert_eq!(report.verdict, Verdict::No);
        let w = report.witness.unwrap();
        assert_eq!(w.value, "a");
        assert_eq!(w.relation, "left_only");
    }

    #[test]
    fn equivalent_no_can_have_right_only_witness() {
        let report = check(Query::Equivalent, "b", "a|b");
        assert_eq!(report.verdict, Verdict::No);
        let w = report.witness.unwrap();
        assert_eq!(w.value, "a");
        assert_eq!(w.relation, "right_only");
    }

    #[test]
    fn includes_agrees_with_automata() {
        for (l, r) in [("a*", "a+"), ("[a-c]+", "[a-c]*"), ("ab", "a|b")] {
            let auto = analyze_binary_with_backend(
                Query::Includes,
                l,
                r,
                &Config::default(),
                &crate::analysis::AutomataBackend,
            )
            .unwrap();
            let hkc = check(Query::Includes, l, r);
            assert_eq!(auto.verdict, hkc.verdict, "{l} includes {r}?");
            let av = auto.witness.as_ref().map(|w| &w.value);
            let hv = hkc.witness.as_ref().map(|w| &w.value);
            assert_eq!(av, hv, "{l} includes {r}: witness mismatch");
        }
    }

    #[test]
    fn overlap_delegates_to_automata_exactly() {
        let auto = analyze_binary_with_backend(
            Query::Overlap,
            "a+b",
            "ab+",
            &Config::default(),
            &crate::analysis::AutomataBackend,
        )
        .unwrap();
        let hkc = check(Query::Overlap, "a+b", "ab+");
        assert_eq!(auto.verdict, hkc.verdict);
        assert_eq!(auto.witness.map(|w| w.value), hkc.witness.map(|w| w.value));
    }

    #[test]
    fn empty_entry_point_matches_automata() {
        let config = Config::default();
        for pattern in ["a*", "a+", "", "a|b"] {
            let auto = crate::analysis::analyze_empty_with_backend(
                pattern,
                &config,
                &crate::analysis::AutomataBackend,
            )
            .unwrap();
            let hkc =
                crate::analysis::analyze_empty_with_backend(pattern, &config, &HkcBackend).unwrap();
            assert_eq!(auto.verdict, hkc.verdict, "empty({pattern:?})");
        }
    }

    /// Window-family stress case, same shape as `antichain.rs`'s own
    /// regression test: confirms HKC's congruence pruning keeps `R` small
    /// on a pattern whose naive subset construction is exponential, and
    /// that the witness it finds agrees with `automata`'s.
    #[test]
    fn window_family_agrees_with_automata() {
        let left = "(a|b)*a(a|b){5}";
        let right = "(a|b)*a(a|b){4}";
        let config = Config {
            max_product_states: 200_000,
            timeout_ms: 20_000,
            ..Config::default()
        };
        let auto = analyze_binary_with_backend(
            Query::Includes,
            left,
            right,
            &config,
            &crate::analysis::AutomataBackend,
        )
        .unwrap();
        let hkc = analyze_binary_with_backend(Query::Includes, left, right, &config, &HkcBackend)
            .unwrap();
        assert_eq!(auto.verdict, Verdict::No);
        assert_eq!(hkc.verdict, Verdict::No);
        assert_eq!(auto.witness.map(|w| w.value), hkc.witness.map(|w| w.value));
    }
}
