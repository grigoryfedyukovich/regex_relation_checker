//! A second [`RelationBackend`] implementation, alongside [`AutomataBackend`]
//! in `analysis.rs`.
//!
//! Where `AutomataBackend` does subset construction lazily, on the fly, as
//! part of a single product-BFS over both patterns at once,
//! [`MinimizedBackend`] takes a different route:
//!
//! 1. Determinize each NFA into a fully materialized DFA (subset
//!    construction run to completion, not on demand).
//! 2. Minimize each DFA independently (Moore-style partition refinement).
//! 3. For `equivalent`: compare the two minimized DFAs via canonical-form
//!    BFS relabeling. Two minimal DFAs recognize the same language if and
//!    only if they are isomorphic (unique-minimal-DFA theorem), so an exact
//!    structural match settles the query *without ever running a product
//!    search* -- a genuinely different technique from `AutomataBackend`,
//!    not just the same algorithm renamed.
//! 4. When that fast path doesn't apply -- the two aren't equivalent, or the
//!    query is `overlap`/`includes` (isomorphism alone can't answer either
//!    of those; both are inherently reachability questions) -- fall back to
//!    a product search over the two *minimized* DFAs. This still needs a
//!    BFS structurally similar to `AutomataBackend`'s, but over materialized
//!    DFA states rather than lazily-computed NFA subsets, and it starts from
//!    a smaller, pre-reduced state space.
//!
//! Keeping both backends around is deliberate: they can be cross-checked
//! against each other (see the tests below, and `tests/backend_agreement.rs`),
//! which is a stronger regression guard than either one alone -- a bug
//! specific to one backend's implementation would very plausibly show up as
//! a disagreement rather than being silently shared by both.
//!
//! Steps 1 and 2 (determinize, minimize) are bounded by both
//! `--max-states` *and* `--timeout-ms`: earlier versions checked only the
//! state cap during those two steps, so a slow determinization or
//! minimization ran to completion no matter how small a timeout was
//! requested -- the deadline only took effect once (if) the code reached
//! the product-search fallback in step 4. `determinize`/`minimize` now take
//! `started`/`deadline` and return `Result<_, BackendStatus>` so a timeout
//! partway through either one is reported the same way a timeout anywhere
//! else in this crate is: `BackendStatus::Timeout`, surfaced as
//! `Verdict::Unknown`.

use crate::analysis::{BackendResult, BackendStatus, Query, RelationBackend};
use crate::charset::{alphabet_partition, representative_chars, CharSet, Interval};
use crate::config::Config;
use crate::nfa::Nfa;
use crate::report::relation;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

/// Reserved index of the dead/sink state every [`Dfa`] carries: non-accepting,
/// and every character maps back to itself. Having an explicit sink makes the
/// transition function total (every state has *some* target for every
/// character), which removes the need for `Option`-handling everywhere a
/// transition is looked up.
pub(crate) const DEAD: usize = 0;

#[derive(Clone, Debug)]
pub(crate) struct DfaState {
    pub(crate) accepting: bool,
    pub(crate) transitions: Vec<(CharSet, usize)>,
    pub(crate) members: Vec<usize>,
}

#[derive(Clone, Debug)]
pub(crate) struct Dfa {
    pub(crate) states: Vec<DfaState>,
    pub(crate) start: usize,
}

/// Look up where `state_id` goes on `ch`, defaulting to [`DEAD`] when no
/// transition covers it.
fn lookup_transition(dfa: &Dfa, state_id: usize, ch: char) -> usize {
    dfa.states[state_id]
        .transitions
        .iter()
        .find(|(set, _)| set.contains(ch))
        .map(|(_, target)| *target)
        .unwrap_or(DEAD)
}

/// Full subset construction, run to completion rather than lazily -- every
/// reachable NFA subset becomes one DFA state, bounded by `max_states`
/// (reusing `config.max_product_states` as the same kind of resource limit
/// `AutomataBackend` respects, just applied to determinization instead of
/// the product search) *and* by `deadline`. Unlike the product-search loops
/// elsewhere in this crate, this used to have no time-based bound at all --
/// only `max_states` -- so `--timeout-ms` had no effect until determinization
/// finished or hit the state cap, however long that took.
pub(crate) fn determinize(
    nfa: &Nfa,
    max_states: usize,
    started: Instant,
    deadline: Duration,
) -> Result<Dfa, BackendStatus> {
    let mut states: Vec<DfaState> = vec![DfaState {
        accepting: false,
        transitions: Vec::new(),
        members: Vec::new(),
    }];
    let mut subsets: Vec<Vec<usize>> = vec![Vec::new()];
    let mut subset_index: HashMap<Vec<usize>, usize> = HashMap::new();
    subset_index.insert(Vec::new(), DEAD);

    let start_subset = nfa.start_subset();
    let start_id = if start_subset.is_empty() {
        DEAD
    } else {
        let id = states.len();
        subset_index.insert(start_subset.clone(), id);
        states.push(DfaState {
            accepting: nfa.is_accepting(&start_subset),
            transitions: Vec::new(),
            members: start_subset.clone(),
        });
        subsets.push(start_subset);
        id
    };

    let mut queue: VecDeque<usize> = VecDeque::new();
    if start_id != DEAD {
        queue.push_back(start_id);
    }

    while let Some(state_id) = queue.pop_front() {
        if started.elapsed() >= deadline {
            return Err(BackendStatus::Timeout);
        }
        let subset = subsets[state_id].clone();
        let outgoing: Vec<&CharSet> = nfa.outgoing_sets(&subset).collect();
        for (start, end, rep) in alphabet_partition(&outgoing) {
            // Checked per transition, not just once per popped state: a
            // single state can partition into many distinct character
            // ranges (especially with `--alphabet unicode`), and that whole
            // batch would otherwise run to completion before the next
            // chance to notice the deadline has passed.
            if started.elapsed() >= deadline {
                return Err(BackendStatus::Timeout);
            }
            let next_subset = nfa.step(&subset, rep);
            let target = if next_subset.is_empty() {
                DEAD
            } else if let Some(&idx) = subset_index.get(&next_subset) {
                idx
            } else {
                if states.len() >= max_states {
                    return Err(BackendStatus::StateLimit);
                }
                let idx = states.len();
                subset_index.insert(next_subset.clone(), idx);
                states.push(DfaState {
                    accepting: nfa.is_accepting(&next_subset),
                    transitions: Vec::new(),
                    members: next_subset.clone(),
                });
                subsets.push(next_subset);
                queue.push_back(idx);
                idx
            };
            if target != DEAD {
                states[state_id].transitions.push((
                    CharSet::from_u32_intervals(vec![Interval::new(start, end)]),
                    target,
                ));
            }
        }
    }

    Ok(Dfa {
        states,
        start: start_id,
    })
}

/// Moore-style partition refinement: start with two classes (accepting /
/// not), then repeatedly split any class whose members don't all transition
/// into the *same* classes for every representative symbol, until nothing
/// changes. What's left is the minimal DFA. Bounded by `deadline`: this used
/// to run to completion unconditionally, ignoring `--timeout-ms` entirely,
/// even though a full refinement pass touches every state on every
/// iteration and iteration count itself scales with the DFA's size.
pub(crate) fn minimize(
    dfa: &Dfa,
    started: Instant,
    deadline: Duration,
) -> Result<Dfa, BackendStatus> {
    let all_sets: Vec<&CharSet> = dfa
        .states
        .iter()
        .flat_map(|s| s.transitions.iter().map(|(set, _)| set))
        .collect();
    let reps = representative_chars(&all_sets);

    let n = dfa.states.len();
    let mut partition: Vec<usize> = dfa
        .states
        .iter()
        .map(|s| if s.accepting { 1 } else { 0 })
        .collect();

    loop {
        if started.elapsed() >= deadline {
            return Err(BackendStatus::Timeout);
        }
        let mut signature_to_id: HashMap<(usize, Vec<usize>), usize> = HashMap::new();
        let mut new_partition = vec![0usize; n];
        for state_id in 0..n {
            // A single refinement pass touches every state; on a large DFA
            // that pass alone can take a while, so check periodically
            // within it too rather than only once per outer iteration.
            // (Not every state: `Instant::now()` is cheap but not free, and
            // this inner loop is the hot path.)
            if state_id % 4096 == 0 && started.elapsed() >= deadline {
                return Err(BackendStatus::Timeout);
            }
            let signature: Vec<usize> = reps
                .iter()
                .map(|&ch| partition[lookup_transition(dfa, state_id, ch)])
                .collect();
            let key = (partition[state_id], signature);
            let next_id = signature_to_id.len();
            let assigned = *signature_to_id.entry(key).or_insert(next_id);
            new_partition[state_id] = assigned;
        }
        if new_partition == partition {
            break;
        }
        partition = new_partition;
    }

    let num_classes = partition.iter().copied().max().map(|m| m + 1).unwrap_or(0);
    let mut representative_of_class: Vec<Option<usize>> = vec![None; num_classes];
    for (state_id, &class) in partition.iter().enumerate() {
        if representative_of_class[class].is_none() {
            representative_of_class[class] = Some(state_id);
        }
    }

    let mut new_states = Vec::with_capacity(num_classes);
    for rep in &representative_of_class {
        let rep_state = rep.expect("every class has at least one member");
        let accepting = dfa.states[rep_state].accepting;
        let transitions = dfa.states[rep_state]
            .transitions
            .iter()
            .map(|(set, target)| (set.clone(), partition[*target]))
            .collect();
        new_states.push(DfaState {
            accepting,
            transitions,
            members: dfa.states[rep_state].members.clone(),
        });
    }

    Ok(Dfa {
        states: new_states,
        start: partition[dfa.start],
    })
}

/// Canonical BFS relabeling: assign state 0 to `start`, then explore
/// transitions in a single fixed order (`reps`, always ascending) so two
/// isomorphic DFAs relabel into *identical* sequences regardless of their
/// original internal state numbering. This works for DFAs specifically
/// (unlike general graphs, where canonical labeling is intractable) because
/// each state's transition function is deterministic: "the next unexplored
/// state reached via representative i" is uniquely determined, so there is
/// only one possible canonical order to discover states in.
fn canonical_form(dfa: &Dfa, reps: &[char]) -> (Vec<bool>, Vec<Vec<usize>>) {
    let mut canon_id: Vec<Option<usize>> = vec![None; dfa.states.len()];
    let mut order: Vec<usize> = Vec::new();
    canon_id[dfa.start] = Some(0);
    order.push(dfa.start);
    let mut queue: VecDeque<usize> = VecDeque::from([dfa.start]);

    while let Some(state_id) = queue.pop_front() {
        for &ch in reps {
            let target = lookup_transition(dfa, state_id, ch);
            if canon_id[target].is_none() {
                canon_id[target] = Some(order.len());
                order.push(target);
                queue.push_back(target);
            }
        }
    }

    let accepting: Vec<bool> = order.iter().map(|&s| dfa.states[s].accepting).collect();
    let transitions: Vec<Vec<usize>> = order
        .iter()
        .map(|&s| {
            reps.iter()
                .map(|&ch| {
                    canon_id[lookup_transition(dfa, s, ch)]
                        .expect("BFS reaches every state used in canonical_form's own traversal")
                })
                .collect()
        })
        .collect();
    (accepting, transitions)
}

/// Two minimal DFAs recognize the same language iff they're isomorphic --
/// checked here by comparing canonical forms built over a shared
/// representative alphabet (fine enough to distinguish anything either DFA's
/// own transitions could possibly distinguish).
fn dfas_isomorphic(left: &Dfa, right: &Dfa) -> bool {
    let all_sets: Vec<&CharSet> = left
        .states
        .iter()
        .chain(right.states.iter())
        .flat_map(|s| s.transitions.iter().map(|(set, _)| set))
        .collect();
    let reps = representative_chars(&all_sets);
    canonical_form(left, &reps) == canonical_form(right, &reps)
}

fn dfa_classify_relation(
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

struct PairNode {
    key: (usize, usize),
    parent: Option<(usize, char)>,
}

fn timed_out(
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

/// Product search over two already-minimized DFAs. Structurally similar to
/// `analysis.rs`'s `search_product` (same BFS-over-a-visited-set shape, same
/// timing/limit conventions, so results and statistics are directly
/// comparable between backends) but over materialized DFA state pairs
/// instead of lazily-computed NFA subset pairs -- no epsilon closures or
/// subset computation needed here, just direct table lookups.
///
/// `started` is threaded in from the caller (rather than starting a fresh
/// clock here) so determinization and minimization time counts toward the
/// same timeout budget and shows up in the reported `analysis_ms`, instead
/// of happening "for free" before the clock starts.
fn dfa_product_search(
    query: Query,
    left: &Dfa,
    right: &Dfa,
    config: &Config,
    started: Instant,
) -> BackendResult {
    let deadline = Duration::from_millis(config.timeout_ms);
    let mut nodes = vec![PairNode {
        key: (left.start, right.start),
        parent: None,
    }];
    let mut visited: HashSet<(usize, usize)> = HashSet::new();
    visited.insert((left.start, right.start));
    let mut queue: VecDeque<usize> = VecDeque::from([0usize]);
    let mut generated_transitions = 0usize;

    while let Some(node_id) = queue.pop_front() {
        if started.elapsed() >= deadline {
            return timed_out(
                BackendStatus::Timeout,
                nodes.len(),
                generated_transitions,
                started,
            );
        }

        let (l_id, r_id) = nodes[node_id].key;
        let left_accepts = left.states[l_id].accepting;
        let right_accepts = right.states[r_id].accepting;
        if let Some(rel) = dfa_classify_relation(query, left_accepts, right_accepts) {
            let analysis_ms = started.elapsed().as_millis();
            let witness_started = Instant::now();
            let mut chars = Vec::new();
            let mut current = node_id;
            while let Some((parent, ch)) = nodes[current].parent {
                chars.push(ch);
                current = parent;
            }
            chars.reverse();
            let witness: String = chars.into_iter().collect();
            let witness_extraction_ms = witness_started.elapsed().as_millis();
            return BackendResult {
                status: BackendStatus::Found,
                witness: Some(witness),
                relation: Some(rel.to_owned()),
                visited_states: nodes.len(),
                generated_transitions,
                analysis_ms,
                witness_extraction_ms,
            };
        }

        let l_sets: Vec<&CharSet> = left.states[l_id]
            .transitions
            .iter()
            .map(|(set, _)| set)
            .collect();
        let r_sets: Vec<&CharSet> = right.states[r_id]
            .transitions
            .iter()
            .map(|(set, _)| set)
            .collect();
        let combined: Vec<&CharSet> = l_sets.into_iter().chain(r_sets).collect();
        for ch in representative_chars(&combined) {
            // Checked per transition, not just once per popped node: a
            // single node can have a large fan-out, and that whole batch
            // would otherwise run to completion before the next chance to
            // notice the deadline has passed.
            if started.elapsed() >= deadline {
                return timed_out(
                    BackendStatus::Timeout,
                    nodes.len(),
                    generated_transitions,
                    started,
                );
            }
            generated_transitions += 1;
            let next_key = (
                lookup_transition(left, l_id, ch),
                lookup_transition(right, r_id, ch),
            );
            if !visited.insert(next_key) {
                continue;
            }
            if nodes.len() >= config.max_product_states {
                return timed_out(
                    BackendStatus::StateLimit,
                    nodes.len(),
                    generated_transitions,
                    started,
                );
            }
            let next_id = nodes.len();
            nodes.push(PairNode {
                key: next_key,
                parent: Some((node_id, ch)),
            });
            queue.push_back(next_id);
        }
    }

    timed_out(
        BackendStatus::Exhausted,
        nodes.len(),
        generated_transitions,
        started,
    )
}

struct SingleNode {
    key: usize,
    parent: Option<(usize, char)>,
}

/// Single-DFA reachability to the nearest accepting state, for the `empty`
/// query -- same shape as `dfa_product_search` above, just over one DFA
/// instead of a pair.
fn dfa_search_single(dfa: &Dfa, config: &Config, started: Instant) -> BackendResult {
    let deadline = Duration::from_millis(config.timeout_ms);
    let mut nodes = vec![SingleNode {
        key: dfa.start,
        parent: None,
    }];
    let mut visited: HashSet<usize> = HashSet::new();
    visited.insert(dfa.start);
    let mut queue: VecDeque<usize> = VecDeque::from([0usize]);
    let mut generated_transitions = 0usize;

    while let Some(node_id) = queue.pop_front() {
        if started.elapsed() >= deadline {
            return timed_out(
                BackendStatus::Timeout,
                nodes.len(),
                generated_transitions,
                started,
            );
        }

        let state_id = nodes[node_id].key;
        if dfa.states[state_id].accepting {
            let analysis_ms = started.elapsed().as_millis();
            let witness_started = Instant::now();
            let mut chars = Vec::new();
            let mut current = node_id;
            while let Some((parent, ch)) = nodes[current].parent {
                chars.push(ch);
                current = parent;
            }
            chars.reverse();
            let witness: String = chars.into_iter().collect();
            let witness_extraction_ms = witness_started.elapsed().as_millis();
            return BackendResult {
                status: BackendStatus::Found,
                witness: Some(witness),
                relation: Some(relation::IN_LANGUAGE.to_owned()),
                visited_states: nodes.len(),
                generated_transitions,
                analysis_ms,
                witness_extraction_ms,
            };
        }

        let sets: Vec<&CharSet> = dfa.states[state_id]
            .transitions
            .iter()
            .map(|(set, _)| set)
            .collect();
        for ch in representative_chars(&sets) {
            // See the matching comment in `dfa_product_search` above.
            if started.elapsed() >= deadline {
                return timed_out(
                    BackendStatus::Timeout,
                    nodes.len(),
                    generated_transitions,
                    started,
                );
            }
            generated_transitions += 1;
            let next = lookup_transition(dfa, state_id, ch);
            if !visited.insert(next) {
                continue;
            }
            if nodes.len() >= config.max_product_states {
                return timed_out(
                    BackendStatus::StateLimit,
                    nodes.len(),
                    generated_transitions,
                    started,
                );
            }
            let next_id = nodes.len();
            nodes.push(SingleNode {
                key: next,
                parent: Some((node_id, ch)),
            });
            queue.push_back(next_id);
        }
    }

    timed_out(
        BackendStatus::Exhausted,
        nodes.len(),
        generated_transitions,
        started,
    )
}

/// A [`RelationBackend`] built on determinize -> minimize -> (isomorphism
/// check, falling back to product search). See the module documentation for
/// why this is a genuinely different technique from [`AutomataBackend`],
/// not just the same algorithm under a new name.
#[derive(Clone, Copy, Debug, Default)]
pub struct MinimizedBackend;

impl RelationBackend for MinimizedBackend {
    fn name(&self) -> &'static str {
        "minimized_dfa"
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
        let started = Instant::now();
        let deadline = Duration::from_millis(config.timeout_ms);
        let left_dfa = match determinize(left, config.max_product_states, started, deadline) {
            Ok(dfa) => dfa,
            Err(status) => return timed_out(status, 0, 0, started),
        };
        let right_dfa = match determinize(right, config.max_product_states, started, deadline) {
            Ok(dfa) => dfa,
            Err(status) => return timed_out(status, left_dfa.states.len(), 0, started),
        };
        let left_min = match minimize(&left_dfa, started, deadline) {
            Ok(dfa) => dfa,
            Err(status) => {
                return timed_out(
                    status,
                    left_dfa.states.len() + right_dfa.states.len(),
                    0,
                    started,
                )
            }
        };
        let right_min = match minimize(&right_dfa, started, deadline) {
            Ok(dfa) => dfa,
            Err(status) => {
                return timed_out(
                    status,
                    left_min.states.len() + right_dfa.states.len(),
                    0,
                    started,
                )
            }
        };

        if query == Query::Equivalent && dfas_isomorphic(&left_min, &right_min) {
            return BackendResult {
                status: BackendStatus::Exhausted,
                witness: None,
                relation: None,
                visited_states: left_min.states.len() + right_min.states.len(),
                generated_transitions: 0,
                analysis_ms: started.elapsed().as_millis(),
                witness_extraction_ms: 0,
            };
        }

        dfa_product_search(query, &left_min, &right_min, config, started)
    }

    fn analyze_empty(&self, nfa: &Nfa, config: &Config) -> BackendResult {
        let started = Instant::now();
        let deadline = Duration::from_millis(config.timeout_ms);
        match determinize(nfa, config.max_product_states, started, deadline) {
            Ok(dfa) => {
                let dfa_states = dfa.states.len();
                match minimize(&dfa, started, deadline) {
                    Ok(minimized) => dfa_search_single(&minimized, config, started),
                    Err(status) => timed_out(status, dfa_states, 0, started),
                }
            }
            Err(status) => timed_out(status, 0, 0, started),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{
        analyze_binary_with_backend, analyze_empty_with_backend, AutomataBackend,
    };
    use crate::report::Verdict;

    fn both_agree(query: Query, left: &str, right: &str) {
        both_agree_with_config(&Config::default(), query, left, right);
    }

    fn both_agree_with_config(config: &Config, query: Query, left: &str, right: &str) {
        let a = analyze_binary_with_backend(query, left, right, config, &AutomataBackend).unwrap();
        let b = analyze_binary_with_backend(query, left, right, config, &MinimizedBackend).unwrap();
        assert_eq!(
            a.verdict, b.verdict,
            "backends disagree on {:?}({:?}, {:?}): automata={:?} minimized={:?}",
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
    fn agrees_with_automata_backend_on_a_broad_corpus() {
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
        ];
        for &(left, right) in cases {
            for query in [Query::Overlap, Query::Includes, Query::Equivalent] {
                both_agree(query, left, right);
            }
        }
    }

    #[test]
    fn agrees_with_automata_backend_on_unicode_boundary_ranges() {
        // Same regression as `alphabet_partition`'s own unit test in
        // `charset.rs` (`alphabet_partition_covers_a_range_ending_at_the_last_scalar_value`),
        // exercised end-to-end through both backends rather than the helper
        // directly -- this is the shape of query that originally surfaced
        // the bug (a class or `.` whose compiled range runs up to U+10FFFF).
        let config = Config {
            alphabet: crate::config::Alphabet::Unicode,
            ..Config::default()
        };
        let max = '\u{10ffff}';
        let e000 = '\u{e000}';

        // `.` denotes the whole alphabet minus newline, which -- under
        // `--alphabet unicode` -- includes U+E000..U+10FFFF. A pattern that
        // explicitly excludes exactly that trailing block is therefore
        // *not* equivalent to `.`; the buggy `minimized` backend disagreed
        // (it saw `.` as if it stopped short of U+10FFFF too, matching the
        // excluded pattern by coincidence).
        let excludes_top_block = format!("[^\n{e000}-{max}]");
        both_agree_with_config(&config, Query::Equivalent, ".", &excludes_top_block);
        both_agree_with_config(&config, Query::Includes, &excludes_top_block, ".");

        // A class whose only member is the maximum scalar value.
        let singleton_max = format!("[{max}]");
        let a = analyze_empty_with_backend(&singleton_max, &config, &AutomataBackend).unwrap();
        let b = analyze_empty_with_backend(&singleton_max, &config, &MinimizedBackend).unwrap();
        assert_eq!(
            a.verdict, b.verdict,
            "empty({singleton_max:?}) disagreement"
        );
        assert_eq!(a.verdict, Verdict::No, "a singleton class is never empty");

        // A range from an ASCII literal up through the maximum scalar value.
        let a_to_max = format!("[a-{max}]");
        both_agree_with_config(&config, Query::Overlap, &a_to_max, &singleton_max);
        both_agree_with_config(&config, Query::Includes, &singleton_max, &a_to_max);
    }

    #[test]
    fn isomorphism_fast_path_finds_equivalence_without_a_witness() {
        let config = Config::default();
        let report = analyze_binary_with_backend(
            Query::Equivalent,
            "a|b",
            "[ab]",
            &config,
            &MinimizedBackend,
        )
        .unwrap();
        assert_eq!(report.verdict, Verdict::Yes);
        assert!(report.witness.is_none());
    }

    #[test]
    fn agrees_on_emptiness() {
        let config = Config::default();
        for pattern in ["a*", "a+", "a{0}", "[^\\d\\D]"] {
            let a = analyze_empty_with_backend(pattern, &config, &AutomataBackend).unwrap();
            let b = analyze_empty_with_backend(pattern, &config, &MinimizedBackend).unwrap();
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
    fn respects_a_tiny_state_limit() {
        let config = Config {
            max_product_states: 1,
            ..Config::default()
        };
        let report =
            analyze_binary_with_backend(Query::Equivalent, "a", "b", &config, &MinimizedBackend)
                .unwrap();
        assert_eq!(report.verdict, Verdict::Unknown);
    }

    #[test]
    fn determinize_and_minimize_respect_timeout_not_just_state_limit() {
        // Regression guard: `determinize` and `minimize` used to have no
        // time-based bound at all -- only `max_states` -- so `--timeout-ms`
        // had zero effect during those phases for `MinimizedBackend`; only
        // the product-search fallback (or the isomorphism check) ever saw
        // the deadline, and only once determinization/minimization had
        // already run to completion or hit the state cap. A `deadline` of
        // zero is already exhausted by the time it's first checked, so this
        // is deterministic -- it doesn't depend on real elapsed wall-clock
        // time being large enough to notice.
        let config = Config::default();
        let expr = crate::parser::parse("a+", &config).unwrap();
        let nfa = crate::nfa::Nfa::from_expr(&expr);
        let started = Instant::now();
        let already_expired = Duration::from_millis(0);

        assert!(matches!(
            determinize(&nfa, config.max_product_states, started, already_expired),
            Err(BackendStatus::Timeout)
        ));

        let real_deadline = Duration::from_millis(config.timeout_ms);
        let dfa = determinize(&nfa, config.max_product_states, started, real_deadline)
            .expect("determinize with a real deadline should succeed on a tiny pattern");
        assert!(matches!(
            minimize(&dfa, started, already_expired),
            Err(BackendStatus::Timeout)
        ));
    }
}
