//! Differential test between the two `RelationBackend` implementations.
//!
//! This is the concrete payoff of keeping `AutomataBackend` and
//! `MinimizedBackend` around side by side: if they ever disagree on a
//! verdict or a witness for the same input, that's a real bug in one of
//! them, caught automatically on every `cargo test` run rather than
//! depending on someone noticing by hand. See `src/minimize.rs`'s module
//! doc for why the two are different enough in technique for this to be a
//! meaningful check rather than the same code path twice.
//!
//! The pattern generator here is deliberately a near-duplicate of the one in
//! `tests/property.rs` rather than a shared helper -- each `tests/*.rs` file
//! compiles as its own separate crate, so sharing code between them cleanly
//! needs a `tests/common/` module; duplicating ~40 lines of self-contained,
//! already-reviewed generator code was the lower-risk choice here.

use proptest::prelude::*;
use regexrel::{
    analyze_binary_with_backend, analyze_empty_with_backend, AutomataBackend, Config,
    MinimizedBackend, Query,
};

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
            let n = take_byte(bytes, pos) % 2 + 2;
            let mut s = String::new();
            for _ in 0..n {
                s.push_str(&build_pattern(bytes, pos, depth + 1));
            }
            s
        }
        3 => {
            let n = take_byte(bytes, pos) % 2 + 2;
            let parts: Vec<String> = (0..n)
                .map(|_| build_pattern(bytes, pos, depth + 1))
                .collect();
            format!("({})", parts.join("|"))
        }
        4 => format!("({})*", build_pattern(bytes, pos, depth + 1)),
        5 => format!("({})+", build_pattern(bytes, pos, depth + 1)),
        6 => format!("({})?", build_pattern(bytes, pos, depth + 1)),
        _ => {
            let lo = (take_byte(bytes, pos) % 3) as usize;
            let extra = (take_byte(bytes, pos) % 3) as usize;
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

proptest! {
    #![proptest_config(ProptestConfig { cases: 200, .. ProptestConfig::default() })]

    #[test]
    fn backends_agree_on_binary_queries(left_bytes in byte_strategy(), right_bytes in byte_strategy()) {
        let left = pattern_from_bytes(&left_bytes);
        let right = pattern_from_bytes(&right_bytes);
        let config = Config::default();

        for query in [Query::Overlap, Query::Includes, Query::Equivalent] {
            let automata = analyze_binary_with_backend(query, &left, &right, &config, &AutomataBackend);
            let minimized = analyze_binary_with_backend(query, &left, &right, &config, &MinimizedBackend);
            if let (Ok(a), Ok(m)) = (automata, minimized) {
                prop_assert_eq!(
                    a.verdict, m.verdict,
                    "{:?}({:?}, {:?}): automata={:?} minimized={:?}",
                    query, left, right, a.verdict, m.verdict
                );
                let a_witness = a.witness.map(|w| w.value);
                let m_witness = m.witness.map(|w| w.value);
                prop_assert_eq!(
                    a_witness, m_witness,
                    "{:?}({:?}, {:?}): witnesses differ between backends",
                    query, left, right
                );
            }
        }
    }

    #[test]
    fn backends_agree_on_emptiness(bytes in byte_strategy()) {
        let pattern = pattern_from_bytes(&bytes);
        let config = Config::default();
        let automata = analyze_empty_with_backend(&pattern, &config, &AutomataBackend);
        let minimized = analyze_empty_with_backend(&pattern, &config, &MinimizedBackend);
        if let (Ok(a), Ok(m)) = (automata, minimized) {
            prop_assert_eq!(
                a.verdict, m.verdict,
                "empty({:?}): automata={:?} minimized={:?}",
                pattern, a.verdict, m.verdict
            );
            let a_witness = a.witness.map(|w| w.value);
            let m_witness = m.witness.map(|w| w.value);
            prop_assert_eq!(
                a_witness, m_witness,
                "empty({:?}): witnesses differ between backends",
                pattern
            );
        }
    }
}
