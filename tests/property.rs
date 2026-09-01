//! Property-based differential tests for `regexrel`.
//!
//! These tests exercise the *real* crate end to end (`regexrel::parse`,
//! `regexrel::analyze_binary`) -- unlike the earlier Python-based audit done
//! for this project, nothing here is a snapshot or a port frozen at a point
//! in time. It always runs against whatever `src/` currently contains, so it
//! keeps catching regressions instead of silently drifting out of sync.
//!
//! Ground truth comes from `positions_after`/`full_match` below: a small,
//! separately-written interpreter that walks the parsed `Expr` AST directly
//! and simulates "the set of cursor positions reachable so far" through it.
//! It shares no code with `nfa.rs` (no Thompson construction, no subset
//! BFS) or `analysis.rs` (no product search) -- a bug in either of those
//! would have to coincidentally reproduce itself here to go undetected,
//! which is the whole point of having an independent check at all.
//!
//! Two kinds of assertions are made, and they are not equally strong:
//!
//! - **Witness validity** (any time `analyze_binary` returns a witness) is
//!   checked unconditionally by replaying that exact witness through
//!   `full_match`. This can never produce a false failure regardless of
//!   pattern complexity or witness length -- it just re-derives whether the
//!   *specific* string returned actually satisfies the claimed relation.
//! - **Absence of a counterexample** (`analyze_binary` claiming NO common
//!   string / NO counterexample / full equivalence, i.e. no witness at all)
//!   is checked by brute-force enumeration up to `MAX_BRUTE_LEN` over a
//!   small closed alphabet. Finding a counterexample within that bound is
//!   decisive (a real bug). *Not* finding one is consistent with, but does
//!   not prove, the claimed verdict -- exactly mirroring the same
//!   brute-force-is-a-spot-check-not-a-proof caveat documented in the
//!   earlier Python-based audit.

use proptest::prelude::*;
use regexrel::ast::{Expr, ExprKind};
use regexrel::{analyze_binary, analyze_match, parse, Config, Query, Verdict};
use std::collections::BTreeSet;

// ---------------------------------------------------------------------
// Byte-driven pattern generator.
//
// Rather than composing proptest's recursive `Strategy` combinators (easy
// to get subtly wrong without a compiler on hand to check against), this
// generates patterns from a plain `Vec<u8>` that proptest shrinks for us:
// each byte drives one grammar decision. The generator is total -- it
// never panics no matter how short the byte vector is (`take_byte` returns
// 0 once exhausted) -- and depth-bounded, so it always terminates.
//
// Deliberately closed universe: every literal/class member ever produced
// comes from `LITERALS` below. That closure is what lets `BRUTE_ALPHABET`
// below be provably sufficient by construction, rather than a hand-picked
// list that has to be remembered and kept in sync as the generator grows
// (which is exactly the mistake made -- and caught -- during the earlier
// Python-based audit of this project).
// ---------------------------------------------------------------------

const LITERALS: [char; 3] = ['a', 'b', 'c'];

fn take_byte(bytes: &[u8], pos: &mut usize) -> u8 {
    let b = bytes.get(*pos).copied().unwrap_or(0);
    *pos += 1;
    b
}

fn build_pattern(bytes: &[u8], pos: &mut usize, depth: u32) -> String {
    let branch_count: u8 = if depth >= 3 { 2 } else { 8 };
    let choice = take_byte(bytes, pos) % branch_count;
    match choice {
        0 => LITERALS[(take_byte(bytes, pos) as usize) % LITERALS.len()].to_string(),
        1 => {
            // A small class over LITERALS, optionally negated. The mask is
            // forced nonzero so the class is never empty.
            let mask = take_byte(bytes, pos) % 7 + 1;
            let negate = take_byte(bytes, pos).is_multiple_of(2);
            let mut members = String::new();
            for (i, ch) in LITERALS.iter().enumerate() {
                if mask & (1 << i) != 0 {
                    members.push(*ch);
                }
            }
            if negate {
                format!("[^{members}]")
            } else {
                format!("[{members}]")
            }
        }
        2 => {
            let n = take_byte(bytes, pos) % 2 + 2; // 2..=3 parts
            let mut s = String::new();
            for _ in 0..n {
                s.push_str(&build_pattern(bytes, pos, depth + 1));
            }
            s
        }
        3 => {
            let n = take_byte(bytes, pos) % 2 + 2; // 2..=3 branches
            let parts: Vec<String> = (0..n)
                .map(|_| build_pattern(bytes, pos, depth + 1))
                .collect();
            format!("({})", parts.join("|"))
        }
        4 => format!("({})*", build_pattern(bytes, pos, depth + 1)),
        5 => format!("({})+", build_pattern(bytes, pos, depth + 1)),
        6 => format!("({})?", build_pattern(bytes, pos, depth + 1)),
        _ => {
            let lo = (take_byte(bytes, pos) % 3) as usize; // 0..=2
            let extra = (take_byte(bytes, pos) % 3) as usize; // 0..=2
            let hi = lo + extra;
            format!("({}){{{lo},{hi}}}", build_pattern(bytes, pos, depth + 1))
        }
    }
}

fn pattern_from_bytes(bytes: &[u8]) -> String {
    let mut pos = 0usize;
    build_pattern(bytes, &mut pos, 0)
}

fn byte_strategy() -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(any::<u8>(), 8..64)
}

/// Candidate strings to test `analyze_match` against, drawn from
/// `BRUTE_ALPHABET` below (defined further down, next to the brute-force
/// search that motivates it) so match-input coverage and counterexample
/// coverage stay over the same alphabet for the same reason: sufficient by
/// construction, since every literal/class member `build_pattern` can ever
/// produce comes from `LITERALS`, and 'z' stands in for everything else.
fn match_input_strategy() -> impl Strategy<Value = String> {
    proptest::collection::vec(any::<u8>(), 0..8).prop_map(|bytes| {
        bytes
            .iter()
            .map(|&b| BRUTE_ALPHABET[b as usize % 4])
            .collect()
    })
}

// ---------------------------------------------------------------------
// Independent ground-truth interpreter (see module doc for why this is
// written the way it is). Operates directly on `Expr`, never touching
// `regexrel::nfa` or `regexrel::analysis`.
// ---------------------------------------------------------------------

fn positions_after(expr: &Expr, chars: &[char], positions: &BTreeSet<usize>) -> BTreeSet<usize> {
    match &expr.kind {
        ExprKind::Empty | ExprKind::AnchorStart | ExprKind::AnchorEnd => positions.clone(),
        ExprKind::Literal(c) => positions
            .iter()
            .copied()
            .filter(|&p| p < chars.len() && chars[p] == *c)
            .map(|p| p + 1)
            .collect(),
        ExprKind::CharSet(set) => positions
            .iter()
            .copied()
            .filter(|&p| p < chars.len() && set.contains(chars[p]))
            .map(|p| p + 1)
            .collect(),
        ExprKind::Concat(parts) => {
            let mut current = positions.clone();
            for part in parts {
                current = positions_after(part, chars, &current);
                if current.is_empty() {
                    break;
                }
            }
            current
        }
        ExprKind::Alt(branches) => {
            let mut out = BTreeSet::new();
            for branch in branches {
                out.extend(positions_after(branch, chars, positions));
            }
            out
        }
        ExprKind::Repeat {
            expr: inner,
            min,
            max,
        } => {
            let mut current = positions.clone();
            for _ in 0..*min {
                current = positions_after(inner, chars, &current);
                if current.is_empty() {
                    return current;
                }
            }
            let mut union = current.clone();
            let mut seen_states: Vec<BTreeSet<usize>> = vec![current.clone()];
            // Bounded either by the explicit `max` or, for unbounded repeats,
            // by the finite space of possible position-sets (there can never
            // be more than chars.len() + 1 distinct reachable subsets worth
            // visiting before a cycle is guaranteed).
            let cap = match max {
                Some(m) => m.saturating_sub(*min),
                None => chars.len() + 2,
            };
            for _ in 0..cap {
                let next = positions_after(inner, chars, &current);
                if next.is_empty() {
                    break;
                }
                union.extend(next.iter().copied());
                if seen_states.contains(&next) {
                    break; // entered a cycle; nothing further can be new
                }
                seen_states.push(next.clone());
                current = next;
            }
            union
        }
        // `ExprKind` is `#[non_exhaustive]` from this (external, since
        // `tests/` is its own compilation unit) crate's point of view, so
        // the compiler requires this arm even though every variant that
        // exists today is already handled above. Reaching it for real would
        // mean the library grew a 9th variant this independent reference
        // interpreter doesn't yet know how to walk -- a real coverage gap
        // to fix here, not something to silently ignore.
        other => unreachable!("positions_after: unhandled ExprKind variant {other:?}"),
    }
}

fn full_match(expr: &Expr, s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    let start: BTreeSet<usize> = [0].into_iter().collect();
    positions_after(expr, &chars, &start).contains(&chars.len())
}

// ---------------------------------------------------------------------
// Brute-force absence check (see module doc: decisive when it finds
// something, inconclusive when it doesn't).
// ---------------------------------------------------------------------

const BRUTE_ALPHABET: [char; 4] = ['a', 'b', 'c', 'z']; // 'z' stands in for
                                                        // "anything outside {a,b,c}", the only distinction a negated class built
                                                        // purely from LITERALS can ever make -- sufficient by construction, not by
                                                        // having remembered to list every character that might come up.
const MAX_BRUTE_LEN: usize = 6;

fn any_counterexample(query: Query, left: &Expr, right: &Expr) -> Option<String> {
    let mut frontier = vec![String::new()];
    let mut words = vec![String::new()];
    for _ in 0..MAX_BRUTE_LEN {
        let mut next = Vec::new();
        for w in &frontier {
            for c in BRUTE_ALPHABET {
                let mut nw = w.clone();
                nw.push(c);
                words.push(nw.clone());
                next.push(nw);
            }
        }
        frontier = next;
    }
    for w in &words {
        let lm = full_match(left, w);
        let rm = full_match(right, w);
        let is_counterexample = match query {
            Query::Overlap => lm && rm,
            Query::Includes => lm && !rm,
            Query::Equivalent => lm != rm,
            Query::Empty | Query::Match => {
                unreachable!("this helper is only used for binary queries")
            }
        };
        if is_counterexample {
            return Some(w.clone());
        }
    }
    None
}

// ---------------------------------------------------------------------
// Properties.
// ---------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig { cases: 200, .. ProptestConfig::default() })]

    /// For every query, whatever `analyze_binary` reports must be
    /// consistent with the independent interpreter above: any witness it
    /// returns must actually satisfy the relation it claims, and any claim
    /// that no counterexample exists must not be contradicted by one we can
    /// find via brute force within `MAX_BRUTE_LEN`.
    #[test]
    fn analyze_binary_agrees_with_independent_interpreter(
        left_bytes in byte_strategy(),
        right_bytes in byte_strategy(),
    ) {
        let left_pattern = pattern_from_bytes(&left_bytes);
        let right_pattern = pattern_from_bytes(&right_bytes);
        let config = Config::default();

        let left_parsed = parse(&left_pattern, &config);
        let right_parsed = parse(&right_pattern, &config);
        prop_assume!(left_parsed.is_ok() && right_parsed.is_ok());
        let left_expr = left_parsed.unwrap();
        let right_expr = right_parsed.unwrap();

        for query in [Query::Overlap, Query::Includes, Query::Equivalent] {
            let report = match analyze_binary(query, &left_pattern, &right_pattern, &config) {
                Ok(r) => r,
                Err(_) => continue,
            };
            if !matches!(report.verdict, Verdict::Yes | Verdict::No) {
                continue; // UNKNOWN / UNSUPPORTED: out of scope for this check
            }

            if let Some(witness) = &report.witness {
                let lm = full_match(&left_expr, &witness.value);
                let rm = full_match(&right_expr, &witness.value);
                let holds = match query {
                    Query::Overlap => lm && rm,
                    Query::Includes => lm && !rm,
                    Query::Equivalent => lm != rm,
                    Query::Empty | Query::Match => unreachable!(),
                };
                prop_assert!(
                    holds,
                    "witness {:?} for {:?}({:?}, {:?}) failed independent replay: left_matches={} right_matches={}",
                    witness.value, query, left_pattern, right_pattern, lm, rm
                );
            } else {
                // No witness means analyze_binary is claiming there is no
                // counterexample. Try to find one anyway.
                if let Some(w) = any_counterexample(query, &left_expr, &right_expr) {
                    prop_assert!(
                        false,
                        "analyze_binary reported {:?} for {:?}({:?}, {:?}) with no witness, \
                         but brute force found a counterexample: {:?}",
                        report.verdict, query, left_pattern, right_pattern, w
                    );
                }
            }
        }
    }

    /// A language is always equivalent to, and includes, itself.
    #[test]
    fn reflexivity(bytes in byte_strategy()) {
        let pattern = pattern_from_bytes(&bytes);
        let config = Config::default();
        prop_assume!(parse(&pattern, &config).is_ok());

        if let Ok(report) = analyze_binary(Query::Equivalent, &pattern, &pattern, &config) {
            prop_assert_ne!(
                report.verdict, Verdict::No,
                "{:?} was not reported equivalent to itself", pattern
            );
        }
        if let Ok(report) = analyze_binary(Query::Includes, &pattern, &pattern, &config) {
            prop_assert_ne!(
                report.verdict, Verdict::No,
                "{:?} was not reported to include itself", pattern
            );
        }
    }

    /// Overlap has no notion of "left" vs "right" -- swapping the arguments
    /// must not change the answer, including the specific witness (the
    /// search order and tie-breaking are deterministic, so a real
    /// left/right asymmetry bug would show up here as a differing witness,
    /// not just a differing verdict).
    #[test]
    fn overlap_is_symmetric(left_bytes in byte_strategy(), right_bytes in byte_strategy()) {
        let left_pattern = pattern_from_bytes(&left_bytes);
        let right_pattern = pattern_from_bytes(&right_bytes);
        let config = Config::default();
        prop_assume!(parse(&left_pattern, &config).is_ok() && parse(&right_pattern, &config).is_ok());

        let forward = analyze_binary(Query::Overlap, &left_pattern, &right_pattern, &config);
        let backward = analyze_binary(Query::Overlap, &right_pattern, &left_pattern, &config);
        if let (Ok(f), Ok(b)) = (forward, backward) {
            prop_assume!(!matches!(f.verdict, Verdict::Unknown | Verdict::Unsupported));
            prop_assume!(!matches!(b.verdict, Verdict::Unknown | Verdict::Unsupported));
            prop_assert_eq!(
                f.verdict, b.verdict,
                "overlap({:?}, {:?}) = {:?} but overlap({:?}, {:?}) = {:?}",
                left_pattern, right_pattern, f.verdict, right_pattern, left_pattern, b.verdict
            );
            let f_value = f.witness.as_ref().map(|w| w.value.clone());
            let b_value = b.witness.as_ref().map(|w| w.value.clone());
            prop_assert_eq!(
                f_value, b_value,
                "overlap witness differs between overlap({:?}, {:?}) and overlap({:?}, {:?})",
                left_pattern, right_pattern, right_pattern, left_pattern
            );
        }
    }

    /// equivalent(A, B) must hold exactly when includes(A, B) and
    /// includes(B, A) both hold -- checked across two independently
    /// implemented `classify_relation` branches in analysis.rs, so a bug
    /// specific to one branch's boolean logic would surface as a
    /// disagreement here even though both ultimately share the same
    /// product-search machinery.
    #[test]
    fn equivalence_matches_mutual_inclusion(left_bytes in byte_strategy(), right_bytes in byte_strategy()) {
        let left_pattern = pattern_from_bytes(&left_bytes);
        let right_pattern = pattern_from_bytes(&right_bytes);
        let config = Config::default();
        prop_assume!(parse(&left_pattern, &config).is_ok() && parse(&right_pattern, &config).is_ok());

        let eq = analyze_binary(Query::Equivalent, &left_pattern, &right_pattern, &config);
        let fwd = analyze_binary(Query::Includes, &left_pattern, &right_pattern, &config);
        let bwd = analyze_binary(Query::Includes, &right_pattern, &left_pattern, &config);
        if let (Ok(eq), Ok(fwd), Ok(bwd)) = (eq, fwd, bwd) {
            let any_inconclusive = [&eq.verdict, &fwd.verdict, &bwd.verdict]
                .iter()
                .any(|v| matches!(v, Verdict::Unknown | Verdict::Unsupported));
            prop_assume!(!any_inconclusive);
            let mutual_inclusion_holds = fwd.verdict == Verdict::Yes && bwd.verdict == Verdict::Yes;
            prop_assert_eq!(
                eq.verdict == Verdict::Yes, mutual_inclusion_holds,
                "equivalent({:?}, {:?}) = {:?} but includes(l,r)={:?} includes(r,l)={:?}",
                left_pattern, right_pattern, eq.verdict, fwd.verdict, bwd.verdict
            );
        }
    }

    /// `analyze_match(pattern, input)` checked against `full_match`, the
    /// same independent AST-walking interpreter the binary-query test above
    /// uses. This doesn't check that `regexrel`'s backends agree with each
    /// other (`tests/backend_agreement.rs` does that) -- it checks the
    /// `match` feature end-to-end against ground truth that shares no code
    /// with the NFA/subset-construction machinery, the Brzozowski residual
    /// machinery, or the Antimirov linear-form machinery.
    #[test]
    fn analyze_match_agrees_with_independent_interpreter(
        bytes in byte_strategy(),
        input in match_input_strategy(),
    ) {
        let pattern = pattern_from_bytes(&bytes);
        let config = Config::default();
        let Ok(expr) = parse(&pattern, &config) else { return Ok(()); };

        // `analyze_match` re-parses `pattern` with the same `config`
        // internally; since that already succeeded once above (`expr`),
        // it cannot fail to parse a second time. So the only way this call
        // can return `Err` here is `AnalyzeError::Internal` -- i.e. exactly
        // the witness-validation safety net (a backend disagreeing with
        // plain NFA replay on this input) firing for real. Fail loudly
        // instead of silently skipping, or this test would quietly stop
        // covering the one failure mode it exists to catch.
        let report = analyze_match(&pattern, &input, &config).unwrap_or_else(|e| {
            panic!(
                "analyze_match({pattern:?}, {input:?}) errored even though the pattern \
                 already parsed successfully: {e}"
            )
        });
        if !matches!(report.verdict, Verdict::Yes | Verdict::No) {
            return Ok(()); // UNKNOWN / UNSUPPORTED: out of scope for this check
        }

        let expected = full_match(&expr, &input);
        let reported = report.verdict == Verdict::Yes;
        prop_assert_eq!(
            reported, expected,
            "match({:?}, {:?}) reported {:?} but the independent interpreter says matches={}",
            pattern, input, report.verdict, expected
        );
    }
}
