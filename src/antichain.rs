//! Antichain inclusion engine (De Wulf et al., CAV'06).
//!
//! Fifth independent [`RelationBackend`]. A is explored as individual NFA
//! states; B is tracked as a set of states. Pairs `(q_A, S_B)` that are
//! subsumed by a previously seen pair with a smaller-or-equal B-set are
//! pruned (minimal antichain; sound for forward counterexample search).
//!
//! Query mapping:
//! - `empty` — [`search_single`] over A
//! - `overlap` — product of individual NFA states (no subsets)
//! - `includes` — `check_included(left, right)`
//! - `equivalent` — both inclusions, interleaved by string length

use crate::analysis::{search_single, BackendResult, BackendStatus, Query, RelationBackend};
use crate::charset::representative_chars;
use crate::config::Config;
use crate::nfa::Nfa;
use crate::report::relation;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// Packed bit-set of NFA state ids.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct BitSet {
    words: Vec<u64>,
}

impl BitSet {
    fn new(n_states: usize) -> Self {
        let n_words = if n_states == 0 {
            0
        } else {
            n_states.div_ceil(64)
        };
        Self {
            words: vec![0u64; n_words],
        }
    }

    fn from_ids(n_states: usize, ids: &[usize]) -> Self {
        let mut s = Self::new(n_states);
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

    fn contains(&self, id: usize) -> bool {
        if self.words.is_empty() {
            return false;
        }
        (self.words[id / 64] & (1u64 << (id % 64))) != 0
    }

    fn is_empty(&self) -> bool {
        self.words.iter().all(|w| *w == 0)
    }

    /// `self ⊆ other`
    fn is_subset(&self, other: &Self) -> bool {
        self.words
            .iter()
            .zip(other.words.iter())
            .all(|(a, b)| (a & !b) == 0)
    }

    fn union_with(&mut self, other: &Self) {
        for (a, b) in self.words.iter_mut().zip(other.words.iter()) {
            *a |= *b;
        }
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
        if self.words.is_empty() {
            return None;
        }
        if !self.started {
            self.started = true;
            self.word_idx = 0;
            self.current = self.words[0];
        }
        loop {
            if self.current == 0 {
                self.word_idx += 1;
                if self.word_idx >= self.words.len() {
                    return None;
                }
                self.current = self.words[self.word_idx];
                continue;
            }
            let bit = self.current.trailing_zeros() as usize;
            self.current &= self.current.wrapping_sub(1);
            return Some(self.word_idx * 64 + bit);
        }
    }
}

/// Per-`q_A` antichain of *minimal* B-sets.
struct Antichain {
    chains: Vec<Vec<BitSet>>,
}

impl Antichain {
    fn new(n_a: usize) -> Self {
        Self {
            chains: vec![Vec::new(); n_a],
        }
    }

    /// True iff some stored `s'` satisfies `s' ⊆ s`.
    fn is_subsumed(&self, qa: usize, s: &BitSet) -> bool {
        self.chains[qa]
            .iter()
            .rev()
            .any(|s_prime| s_prime.is_subset(s))
    }

    fn insert(&mut self, qa: usize, s: BitSet) {
        let entry = &mut self.chains[qa];
        entry.retain(|old| !s.is_subset(old));
        entry.push(s);
    }

    fn size(&self) -> usize {
        self.chains.iter().map(Vec::len).sum()
    }
}

struct Prepared<'a> {
    n_states: usize,
    accept: usize,
    start_ids: Vec<usize>,
    start_set: BitSet,
    /// `delta[q * n_parts + p]` = ε-closure of char-successors of `q` on part `p`
    delta: Vec<BitSet>,
    n_parts: usize,
    _nfa: &'a Nfa,
}

impl<'a> Prepared<'a> {
    fn new(nfa: &'a Nfa, parts: &[char]) -> Self {
        let n_states = nfa.states.len();
        let n_parts = parts.len();
        let mut eps: Vec<BitSet> = Vec::with_capacity(n_states);
        for q in 0..n_states {
            let closed = nfa.epsilon_closure(std::iter::once(q));
            eps.push(BitSet::from_ids(n_states, &closed));
        }
        let mut delta = Vec::new();
        if n_parts > 0 {
            delta = vec![BitSet::new(n_states); n_states * n_parts];
            for q in 0..n_states {
                for (p, &ch) in parts.iter().enumerate() {
                    let mut post = BitSet::new(n_states);
                    for tr in &nfa.states[q].transitions {
                        if tr.set.contains(ch) {
                            post.union_with(&eps[tr.target]);
                        }
                    }
                    delta[q * n_parts + p] = post;
                }
            }
        }
        let start_ids = nfa.start_subset();
        let start_set = BitSet::from_ids(n_states, &start_ids);
        Self {
            n_states,
            accept: nfa.accept,
            start_ids,
            start_set,
            delta,
            n_parts,
            _nfa: nfa,
        }
    }

    fn delta(&self, q: usize, p: usize) -> &BitSet {
        &self.delta[q * self.n_parts + p]
    }

    fn accepts(&self, q: usize) -> bool {
        q == self.accept
    }

    fn set_accepts(&self, s: &BitSet) -> bool {
        s.contains(self.accept)
    }
}

fn global_parts(left: &Nfa, right: &Nfa) -> Vec<char> {
    let mut sets = Vec::new();
    for nfa in [left, right] {
        for state in &nfa.states {
            for tr in &state.transitions {
                sets.push(&tr.set);
            }
        }
    }
    representative_chars(&sets)
}

fn post_cached(
    eng: &Prepared,
    sb: &BitSet,
    p: usize,
    cache: &mut HashMap<(BitSet, usize), BitSet>,
) -> BitSet {
    if let Some(hit) = cache.get(&(sb.clone(), p)) {
        return hit.clone();
    }
    let mut res = BitSet::new(eng.n_states);
    for s in sb.iter() {
        res.union_with(eng.delta(s, p));
    }
    cache.insert((sb.clone(), p), res.clone());
    res
}

struct Node {
    qa: usize,
    sb: BitSet,
    parent: Option<usize>,
    via: Option<char>,
}

fn reconstruct(arena: &[Node], mut id: usize, extra: Option<char>) -> String {
    let mut chars = Vec::new();
    if let Some(ch) = extra {
        chars.push(ch);
    }
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

/// `L(A) ⊆ L(B)`? Found = counterexample (`left_only`).
fn check_included(a: &Nfa, b: &Nfa, config: &Config) -> BackendResult {
    let started = Instant::now();
    let deadline = Duration::from_millis(config.timeout_ms);
    let parts = global_parts(a, b);
    let eng_a = Prepared::new(a, &parts);
    let eng_b = Prepared::new(b, &parts);
    let mut cache: HashMap<(BitSet, usize), BitSet> = HashMap::new();
    let mut antichain = Antichain::new(eng_a.n_states);
    let mut arena: Vec<Node> = Vec::new();
    let mut generated = 0usize;
    let s0 = eng_b.start_set.clone();

    for &qa in &eng_a.start_ids {
        if eng_a.accepts(qa) && !eng_b.set_accepts(&s0) {
            return found(String::new(), relation::LEFT_ONLY, 1, generated, started);
        }
    }

    let mut current: Vec<usize> = Vec::new();
    for &qa in &eng_a.start_ids {
        if antichain.is_subsumed(qa, &s0) {
            continue;
        }
        if arena.len() >= config.max_product_states {
            return stopped(BackendStatus::StateLimit, arena.len(), generated, started);
        }
        antichain.insert(qa, s0.clone());
        let id = arena.len();
        arena.push(Node {
            qa,
            sb: s0.clone(),
            parent: None,
            via: None,
        });
        current.push(id);
    }

    while !current.is_empty() {
        if started.elapsed() >= deadline {
            return stopped(BackendStatus::Timeout, arena.len(), generated, started);
        }
        let mut next: Vec<usize> = Vec::new();
        for (p, &ch) in parts.iter().enumerate() {
            if started.elapsed() >= deadline {
                return stopped(BackendStatus::Timeout, arena.len(), generated, started);
            }
            for &id in &current {
                generated += 1;
                let qa = arena[id].qa;
                let qa_post = eng_a.delta(qa, p);
                if qa_post.is_empty() {
                    continue;
                }
                let sb_post = post_cached(&eng_b, &arena[id].sb, p, &mut cache);
                for qa2 in qa_post.iter() {
                    if eng_a.accepts(qa2) && !eng_b.set_accepts(&sb_post) {
                        let witness = reconstruct(&arena, id, Some(ch));
                        return found(
                            witness,
                            relation::LEFT_ONLY,
                            arena.len() + 1,
                            generated,
                            started,
                        );
                    }
                    if antichain.is_subsumed(qa2, &sb_post) {
                        continue;
                    }
                    if arena.len() >= config.max_product_states {
                        return stopped(BackendStatus::StateLimit, arena.len(), generated, started);
                    }
                    antichain.insert(qa2, sb_post.clone());
                    let nid = arena.len();
                    arena.push(Node {
                        qa: qa2,
                        sb: sb_post.clone(),
                        parent: Some(id),
                        via: Some(ch),
                    });
                    next.push(nid);
                }
            }
        }
        current = next;
    }

    let _ = antichain.size(); // retained for future --stats wiring
    stopped(BackendStatus::Exhausted, arena.len(), generated, started)
}

struct PairNode {
    qa: usize,
    qb: usize,
    parent: Option<usize>,
    via: Option<char>,
}

fn reconstruct_pair(arena: &[PairNode], mut id: usize, extra: Option<char>) -> String {
    let mut chars = Vec::new();
    if let Some(ch) = extra {
        chars.push(ch);
    }
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

fn check_overlap(a: &Nfa, b: &Nfa, config: &Config) -> BackendResult {
    let started = Instant::now();
    let deadline = Duration::from_millis(config.timeout_ms);
    let parts = global_parts(a, b);
    let eng_a = Prepared::new(a, &parts);
    let eng_b = Prepared::new(b, &parts);

    for &qa in &eng_a.start_ids {
        for &qb in &eng_b.start_ids {
            if eng_a.accepts(qa) && eng_b.accepts(qb) {
                return found(String::new(), relation::IN_BOTH, 1, 0, started);
            }
        }
    }

    let mut arena: Vec<PairNode> = Vec::new();
    let mut seen: HashSet<(u32, u32)> = HashSet::new();
    let mut current: Vec<usize> = Vec::new();
    for &qa in &eng_a.start_ids {
        for &qb in &eng_b.start_ids {
            if !seen.insert((qa as u32, qb as u32)) {
                continue;
            }
            if arena.len() >= config.max_product_states {
                return stopped(BackendStatus::StateLimit, arena.len(), 0, started);
            }
            let id = arena.len();
            arena.push(PairNode {
                qa,
                qb,
                parent: None,
                via: None,
            });
            current.push(id);
        }
    }

    let mut generated = 0usize;
    while !current.is_empty() {
        if started.elapsed() >= deadline {
            return stopped(BackendStatus::Timeout, arena.len(), generated, started);
        }
        let mut next: Vec<usize> = Vec::new();
        for (p, &ch) in parts.iter().enumerate() {
            if started.elapsed() >= deadline {
                return stopped(BackendStatus::Timeout, arena.len(), generated, started);
            }
            for &id in &current {
                generated += 1;
                let qa = arena[id].qa;
                let qb = arena[id].qb;
                let a_post = eng_a.delta(qa, p);
                if a_post.is_empty() {
                    continue;
                }
                let b_post = eng_b.delta(qb, p);
                if b_post.is_empty() {
                    continue;
                }
                for qa2 in a_post.iter() {
                    for qb2 in b_post.iter() {
                        if eng_a.accepts(qa2) && eng_b.accepts(qb2) {
                            let witness = reconstruct_pair(&arena, id, Some(ch));
                            return found(
                                witness,
                                relation::IN_BOTH,
                                arena.len() + 1,
                                generated,
                                started,
                            );
                        }
                        if !seen.insert((qa2 as u32, qb2 as u32)) {
                            continue;
                        }
                        if arena.len() >= config.max_product_states {
                            return stopped(
                                BackendStatus::StateLimit,
                                arena.len(),
                                generated,
                                started,
                            );
                        }
                        let nid = arena.len();
                        arena.push(PairNode {
                            qa: qa2,
                            qb: qb2,
                            parent: Some(id),
                            via: Some(ch),
                        });
                        next.push(nid);
                    }
                }
            }
        }
        current = next;
    }

    stopped(BackendStatus::Exhausted, arena.len(), generated, started)
}

fn check_equivalent(a: &Nfa, b: &Nfa, config: &Config) -> BackendResult {
    let started = Instant::now();
    let deadline = Duration::from_millis(config.timeout_ms);
    let parts = global_parts(a, b);
    let eng_a = Prepared::new(a, &parts);
    let eng_b = Prepared::new(b, &parts);

    let a_eps_acc = eng_a.start_ids.iter().any(|&q| eng_a.accepts(q));
    let b_eps_acc = eng_b.start_ids.iter().any(|&q| eng_b.accepts(q));
    if a_eps_acc && !b_eps_acc {
        return found(String::new(), relation::LEFT_ONLY, 1, 0, started);
    }
    if !a_eps_acc && b_eps_acc {
        return found(String::new(), relation::RIGHT_ONLY, 1, 0, started);
    }

    let mut cache_ab: HashMap<(BitSet, usize), BitSet> = HashMap::new();
    let mut cache_ba: HashMap<(BitSet, usize), BitSet> = HashMap::new();
    let mut chain_ab = Antichain::new(eng_a.n_states);
    let mut chain_ba = Antichain::new(eng_b.n_states);
    let mut arena_ab: Vec<Node> = Vec::new();
    let mut arena_ba: Vec<Node> = Vec::new();
    let mut generated = 0usize;

    let mut current_ab: Vec<usize> = Vec::new();
    let mut current_ba: Vec<usize> = Vec::new();
    for &qa in &eng_a.start_ids {
        if chain_ab.is_subsumed(qa, &eng_b.start_set) {
            continue;
        }
        if arena_ab.len() + arena_ba.len() >= config.max_product_states {
            return stopped(
                BackendStatus::StateLimit,
                arena_ab.len() + arena_ba.len(),
                generated,
                started,
            );
        }
        chain_ab.insert(qa, eng_b.start_set.clone());
        let id = arena_ab.len();
        arena_ab.push(Node {
            qa,
            sb: eng_b.start_set.clone(),
            parent: None,
            via: None,
        });
        current_ab.push(id);
    }
    for &qb in &eng_b.start_ids {
        if chain_ba.is_subsumed(qb, &eng_a.start_set) {
            continue;
        }
        if arena_ab.len() + arena_ba.len() >= config.max_product_states {
            return stopped(
                BackendStatus::StateLimit,
                arena_ab.len() + arena_ba.len(),
                generated,
                started,
            );
        }
        chain_ba.insert(qb, eng_a.start_set.clone());
        let id = arena_ba.len();
        arena_ba.push(Node {
            qa: qb,
            sb: eng_a.start_set.clone(),
            parent: None,
            via: None,
        });
        current_ba.push(id);
    }

    while !current_ab.is_empty() || !current_ba.is_empty() {
        if started.elapsed() >= deadline {
            return stopped(
                BackendStatus::Timeout,
                arena_ab.len() + arena_ba.len(),
                generated,
                started,
            );
        }
        let mut next_ab: Vec<usize> = Vec::new();
        let mut next_ba: Vec<usize> = Vec::new();
        for (p, &ch) in parts.iter().enumerate() {
            if started.elapsed() >= deadline {
                return stopped(
                    BackendStatus::Timeout,
                    arena_ab.len() + arena_ba.len(),
                    generated,
                    started,
                );
            }
            for &id in &current_ab {
                generated += 1;
                let qa = arena_ab[id].qa;
                let qa_post = eng_a.delta(qa, p);
                if qa_post.is_empty() {
                    continue;
                }
                let sb_post = post_cached(&eng_b, &arena_ab[id].sb, p, &mut cache_ab);
                for qa2 in qa_post.iter() {
                    if eng_a.accepts(qa2) && !eng_b.set_accepts(&sb_post) {
                        let witness = reconstruct(&arena_ab, id, Some(ch));
                        return found(
                            witness,
                            relation::LEFT_ONLY,
                            arena_ab.len() + arena_ba.len() + 1,
                            generated,
                            started,
                        );
                    }
                    if chain_ab.is_subsumed(qa2, &sb_post) {
                        continue;
                    }
                    if arena_ab.len() + arena_ba.len() >= config.max_product_states {
                        return stopped(
                            BackendStatus::StateLimit,
                            arena_ab.len() + arena_ba.len(),
                            generated,
                            started,
                        );
                    }
                    chain_ab.insert(qa2, sb_post.clone());
                    let nid = arena_ab.len();
                    arena_ab.push(Node {
                        qa: qa2,
                        sb: sb_post.clone(),
                        parent: Some(id),
                        via: Some(ch),
                    });
                    next_ab.push(nid);
                }
            }
            for &id in &current_ba {
                generated += 1;
                let qb = arena_ba[id].qa;
                let qb_post = eng_b.delta(qb, p);
                if qb_post.is_empty() {
                    continue;
                }
                let sa_post = post_cached(&eng_a, &arena_ba[id].sb, p, &mut cache_ba);
                for qb2 in qb_post.iter() {
                    if eng_b.accepts(qb2) && !eng_a.set_accepts(&sa_post) {
                        let witness = reconstruct(&arena_ba, id, Some(ch));
                        return found(
                            witness,
                            relation::RIGHT_ONLY,
                            arena_ab.len() + arena_ba.len() + 1,
                            generated,
                            started,
                        );
                    }
                    if chain_ba.is_subsumed(qb2, &sa_post) {
                        continue;
                    }
                    if arena_ab.len() + arena_ba.len() >= config.max_product_states {
                        return stopped(
                            BackendStatus::StateLimit,
                            arena_ab.len() + arena_ba.len(),
                            generated,
                            started,
                        );
                    }
                    chain_ba.insert(qb2, sa_post.clone());
                    let nid = arena_ba.len();
                    arena_ba.push(Node {
                        qa: qb2,
                        sb: sa_post.clone(),
                        parent: Some(id),
                        via: Some(ch),
                    });
                    next_ba.push(nid);
                }
            }
        }
        current_ab = next_ab;
        current_ba = next_ba;
    }

    stopped(
        BackendStatus::Exhausted,
        arena_ab.len() + arena_ba.len(),
        generated,
        started,
    )
}

fn run_binary(query: Query, left: &Nfa, right: &Nfa, config: &Config) -> BackendResult {
    match query {
        Query::Overlap => check_overlap(left, right, config),
        Query::Includes => check_included(left, right, config),
        Query::Equivalent => check_equivalent(left, right, config),
        Query::Empty | Query::Match => unreachable!("binary entry with a unary query"),
    }
}

/// Antichain inclusion / NFA-product engine.
#[derive(Clone, Copy, Debug, Default)]
pub struct AntichainBackend;

impl RelationBackend for AntichainBackend {
    fn name(&self) -> &'static str {
        "antichain"
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
    use crate::analysis::{analyze_binary_with_backend, AutomataBackend};
    use crate::parser::parse;
    use crate::report::Verdict;

    fn compile(pattern: &str) -> Nfa {
        Nfa::from_expr(&parse(pattern, &Config::default()).unwrap())
    }

    fn cfg() -> Config {
        Config::default()
    }

    #[test]
    fn bitset_subset_and_union() {
        let mut a = BitSet::new(8);
        a.insert(1);
        a.insert(3);
        let mut b = BitSet::new(8);
        b.insert(1);
        b.insert(3);
        b.insert(5);
        assert!(a.is_subset(&b));
        assert!(!b.is_subset(&a));
        a.union_with(&b);
        assert!(b.is_subset(&a));
        assert_eq!(a.iter().collect::<Vec<_>>(), vec![1, 3, 5]);
    }

    #[test]
    fn plus_is_included_in_star_concat() {
        let report =
            analyze_binary_with_backend(Query::Includes, "a+", "aa*", &cfg(), &AntichainBackend)
                .unwrap();
        assert_eq!(report.verdict, Verdict::Yes);
        assert!(report.witness.is_none());
    }

    #[test]
    fn window_three_not_in_window_two() {
        let report = analyze_binary_with_backend(
            Query::Includes,
            "(a|b)*a(a|b){3}",
            "(a|b)*a(a|b){2}",
            &cfg(),
            &AntichainBackend,
        )
        .unwrap();
        assert_eq!(report.verdict, Verdict::No);
        let witness = report.witness.unwrap();
        assert_eq!(witness.relation, relation::LEFT_ONLY);
        assert_eq!(witness.value.chars().count(), 4);
        let nfa_left = compile("(a|b)*a(a|b){3}");
        let nfa_right = compile("(a|b)*a(a|b){2}");
        assert!(nfa_left.matches(&witness.value));
        assert!(!nfa_right.matches(&witness.value));
    }

    /// Correctness on the classic exponential window family.
    ///
    /// Raw `visited_product_states` is not compared to the automata backend
    /// here: the two engines count different state spaces (individual A-states
    /// × B-sets vs subset×subset), and without the simulation preorder
    /// (Abdulla et al.) basic antichain can explore more nodes while still
    /// returning the right counterexample. Simulation is the planned follow-up
    /// that makes the size advantage decisive.
    #[test]
    fn window_family_counterexample() {
        let left = "(a|b)*a(a|b){8}";
        let right = "(a|b)*a(a|b){7}";
        let config = Config {
            max_product_states: 1_000_000,
            timeout_ms: 30_000,
            ..Config::default()
        };
        let anti =
            analyze_binary_with_backend(Query::Includes, left, right, &config, &AntichainBackend)
                .unwrap();
        let auto =
            analyze_binary_with_backend(Query::Includes, left, right, &config, &AutomataBackend)
                .unwrap();
        assert_eq!(anti.verdict, Verdict::No);
        assert_eq!(auto.verdict, Verdict::No);
        let w = anti
            .witness
            .as_ref()
            .expect("antichain should return a witness");
        assert_eq!(w.relation, relation::LEFT_ONLY);
        let nfa_left = compile(left);
        let nfa_right = compile(right);
        assert!(nfa_left.matches(&w.value));
        assert!(!nfa_right.matches(&w.value));
        // Shortest distinguishing suffix has length 9 for window 8 vs 7.
        assert_eq!(w.value.chars().count(), 9);
    }

    #[test]
    fn overlap_agrees_with_automata() {
        let anti =
            analyze_binary_with_backend(Query::Overlap, "a+b", "ab+", &cfg(), &AntichainBackend)
                .unwrap();
        let auto =
            analyze_binary_with_backend(Query::Overlap, "a+b", "ab+", &cfg(), &AutomataBackend)
                .unwrap();
        assert_eq!(anti.verdict, auto.verdict);
        assert_eq!(
            anti.witness.as_ref().map(|w| &w.value),
            auto.witness.as_ref().map(|w| &w.value)
        );
    }

    #[test]
    fn equivalent_yes_and_no() {
        let yes =
            analyze_binary_with_backend(Query::Equivalent, "a+", "aa*", &cfg(), &AntichainBackend)
                .unwrap();
        assert_eq!(yes.verdict, Verdict::Yes);

        let no =
            analyze_binary_with_backend(Query::Equivalent, "a|b", "b", &cfg(), &AntichainBackend)
                .unwrap();
        assert_eq!(no.verdict, Verdict::No);
        assert_eq!(no.witness.as_ref().unwrap().value, "a");
        assert_eq!(no.witness.as_ref().unwrap().relation, relation::LEFT_ONLY);
    }

    #[test]
    fn empty_string_counterexample() {
        let report =
            analyze_binary_with_backend(Query::Includes, "a*", "a+", &cfg(), &AntichainBackend)
                .unwrap();
        assert_eq!(report.verdict, Verdict::No);
        assert_eq!(report.witness.unwrap().value, "");
    }
}
