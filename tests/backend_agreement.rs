//! Differential test between the `RelationBackend` implementations.
//!
//! This is the concrete payoff of keeping `AutomataBackend`,
//! `MinimizedBackend`, `DerivativeBackend`, and `AntimirovBackend` around
//! side by side: if they ever disagree on a verdict or a witness for the
//! same input, that's a real bug in one of them, caught automatically on
//! every `cargo test` run rather than depending on someone noticing by
//! hand. `AntimirovBackend` and `DerivativeBackend` share the same residual
//! algebra (same `Reg` shape, same normalization rules) but
//! `AntimirovBackend` decides acceptance over *sets* of residuals instead
//! of one combined residual, so a divergence between the two specifically
//! implicates the linear-form/set bookkeeping rather than the shared
//! algebra.
//!
//! This file was reconstructed after `tests/` went missing from a repo
//! snapshot handed off mid-project; see the `match_input` coverage below in
//! particular, which is new -- that trait method has four independent
//! implementations (the shared NFA-walk default, plus overrides in
//! `DerivativeBackend` and `AntimirovBackend`) and had no cross-backend
//! check anywhere before this.

use proptest::prelude::*;
use regexrel::{
    analyze_binary_with_backend, analyze_empty_with_backend, analyze_match_with_backend,
    AntimirovBackend, AutomataBackend, Config, DerivativeBackend, MinimizedBackend, Query,
    RelationBackend,
};

const AUTOMATA: AutomataBackend = AutomataBackend;
const MINIMIZED: MinimizedBackend = MinimizedBackend;
const DERIVATIVES: DerivativeBackend = DerivativeBackend;
const ANTIMIROV: AntimirovBackend = AntimirovBackend;

/// Every backend under test, as trait objects, so callers can just loop
/// instead of repeating the same four calls by hand. `const` (rather than
/// building `&AutomataBackend` etc. inline) sidesteps any question about
/// whether an inline unit-struct borrow gets promoted to `'static` in this
/// position -- a `const` reference is unambiguously `'static`.
fn backends() -> [&'static dyn RelationBackend; 4] {
    [&AUTOMATA, &MINIMIZED, &DERIVATIVES, &ANTIMIROV]
}

/// Small, bounded regex-string generator. Every leaf and every combinator
/// below produces syntax this crate's parser accepts -- alternation, star,
/// optional, and plus all wrap their operand in parens, so precedence is
/// never ambiguous regardless of how deeply this recurses. Bounded to depth
/// 4 / 64 total nodes so generated patterns stay small enough that a
/// disagreement is easy to read out of a proptest failure, and so the
/// `Match` differential test (which runs every backend on every generated
/// pattern *and* input) stays fast.
fn arb_pattern() -> impl Strategy<Value = String> {
    let leaf = prop_oneof![
        Just("a".to_string()),
        Just("b".to_string()),
        Just("c".to_string()),
        Just("".to_string()),
        Just("[a-c]".to_string()),
    ];
    leaf.prop_recursive(4, 64, 4, |inner| {
        prop_oneof![
            (inner.clone(), inner.clone()).prop_map(|(a, b)| format!("{a}{b}")),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| format!("({a}|{b})")),
            inner.clone().prop_map(|a| format!("({a})*")),
            inner.clone().prop_map(|a| format!("({a})?")),
            inner.prop_map(|a| format!("({a})+")),
        ]
    })
}

/// Short strings over the same alphabet the patterns above draw from, so a
/// meaningful fraction of generated (pattern, input) pairs actually match
/// rather than missing the alphabet entirely.
fn arb_input() -> impl Strategy<Value = String> {
    proptest::collection::vec(prop_oneof![Just('a'), Just('b'), Just('c')], 0..8)
        .prop_map(|chars| chars.into_iter().collect())
}

proptest! {
    #[test]
    fn all_backends_agree_on_binary_queries(left in arb_pattern(), right in arb_pattern()) {
        let config = Config::default();
        let names = backends().map(|b| b.name());
        for query in [Query::Overlap, Query::Includes, Query::Equivalent] {
            let results: Vec<_> = backends()
                .iter()
                .map(|backend| analyze_binary_with_backend(query, &left, &right, &config, *backend))
                .collect();
            // A pattern that fails to parse fails identically for every
            // backend (parsing happens once, before backend dispatch), so
            // there's nothing to compare in that case -- skip rather than
            // treat the shared parse error as a disagreement.
            let Ok(reports) = results.into_iter().collect::<Result<Vec<_>, _>>() else {
                continue;
            };
            for i in 1..reports.len() {
                prop_assert_eq!(
                    reports[0].verdict, reports[i].verdict,
                    "{:?}({:?}, {:?}): {}={:?} {}={:?}",
                    query, left, right,
                    names[0], reports[0].verdict, names[i], reports[i].verdict
                );
                let baseline_witness = reports[0].witness.as_ref().map(|w| &w.value);
                let this_witness = reports[i].witness.as_ref().map(|w| &w.value);
                prop_assert_eq!(
                    baseline_witness, this_witness,
                    "{:?}({:?}, {:?}): witnesses differ {} vs {}",
                    query, left, right, names[0], names[i]
                );
            }
        }
    }

    #[test]
    fn all_backends_agree_on_emptiness(pattern in arb_pattern()) {
        let config = Config::default();
        let names = backends().map(|b| b.name());
        let results: Vec<_> = backends()
            .iter()
            .map(|backend| analyze_empty_with_backend(&pattern, &config, *backend))
            .collect();
        let Ok(reports) = results.into_iter().collect::<Result<Vec<_>, _>>() else {
            return Ok(());
        };
        for i in 1..reports.len() {
            prop_assert_eq!(
                reports[0].verdict, reports[i].verdict,
                "empty({:?}): {}={:?} {}={:?}",
                pattern, names[0], reports[0].verdict, names[i], reports[i].verdict
            );
            let baseline_witness = reports[0].witness.as_ref().map(|w| &w.value);
            let this_witness = reports[i].witness.as_ref().map(|w| &w.value);
            prop_assert_eq!(
                baseline_witness, this_witness,
                "empty({:?}): witnesses differ {} vs {}",
                pattern, names[0], names[i]
            );
        }
    }

    #[test]
    fn all_backends_agree_on_match(pattern in arb_pattern(), input in arb_input()) {
        let config = Config::default();
        let names = backends().map(|b| b.name());
        let results: Vec<_> = backends()
            .iter()
            .map(|backend| analyze_match_with_backend(&pattern, &input, &config, *backend))
            .collect();
        let Ok(reports) = results.into_iter().collect::<Result<Vec<_>, _>>() else {
            return Ok(());
        };
        for i in 1..reports.len() {
            prop_assert_eq!(
                reports[0].verdict, reports[i].verdict,
                "match({:?}, {:?}): {}={:?} {}={:?}",
                pattern, input, names[0], reports[0].verdict, names[i], reports[i].verdict
            );
        }
    }
}
