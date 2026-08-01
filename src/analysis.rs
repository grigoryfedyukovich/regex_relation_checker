use crate::config::Config;
use crate::nfa::Nfa;
use crate::parser::{parse, FrontendError, FrontendErrorKind};
use crate::report::{
    relation, BackendInfo, Diagnostic, Report, Semantics, Statistics, Timings, Verdict, Witness,
};
use serde::Serialize;
use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Query {
    Empty,
    Overlap,
    Includes,
    Equivalent,
}

#[derive(Debug, Error)]
pub enum AnalyzeError {
    #[error(transparent)]
    Input(#[from] FrontendError),
    #[error("internal witness validation failed: {0}")]
    Internal(String),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ProductKey {
    left: Vec<usize>,
    right: Vec<usize>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct SubsetKey(Vec<usize>);

#[derive(Clone, Debug)]
struct SearchNode<K> {
    key: K,
    parent: Option<(usize, char)>,
}

#[derive(Clone, Debug)]
pub struct BackendResult {
    pub status: BackendStatus,
    pub witness: Option<String>,
    pub relation: Option<String>,
    pub visited_states: usize,
    pub generated_transitions: usize,
    pub analysis_ms: u128,
    pub witness_extraction_ms: u128,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendStatus {
    Found,
    Exhausted,
    StateLimit,
    Timeout,
}

pub trait RelationBackend {
    fn name(&self) -> &'static str;

    fn version(&self) -> &'static str;

    fn analyze_binary(
        &self,
        query: Query,
        left: &Nfa,
        right: &Nfa,
        config: &Config,
    ) -> BackendResult;

    fn analyze_empty(&self, nfa: &Nfa, config: &Config) -> BackendResult;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AutomataBackend;

impl RelationBackend for AutomataBackend {
    fn name(&self) -> &'static str {
        "in_process_automata"
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
        search_product(query, left, right, config)
    }

    fn analyze_empty(&self, nfa: &Nfa, config: &Config) -> BackendResult {
        search_single(nfa, config)
    }
}

pub fn analyze_binary(
    query: Query,
    left_pattern: &str,
    right_pattern: &str,
    config: &Config,
) -> Result<Report, AnalyzeError> {
    analyze_binary_with_backend(query, left_pattern, right_pattern, config, &AutomataBackend)
}

pub fn analyze_binary_with_backend(
    query: Query,
    left_pattern: &str,
    right_pattern: &str,
    config: &Config,
    backend: &dyn RelationBackend,
) -> Result<Report, AnalyzeError> {
    if !matches!(query, Query::Overlap | Query::Includes | Query::Equivalent) {
        return Err(AnalyzeError::Internal(
            "binary analysis called with the empty-language query".to_owned(),
        ));
    }
    let parse_started = Instant::now();
    let left_expr = match parse(left_pattern, config) {
        Ok(expr) => expr,
        Err(error) if error.kind == FrontendErrorKind::Unsupported => {
            return Ok(unsupported_report(
                query,
                "left",
                error,
                config,
                parse_started.elapsed(),
                backend,
            ));
        }
        Err(error) => return Err(error.into()),
    };
    let right_expr = match parse(right_pattern, config) {
        Ok(expr) => expr,
        Err(error) if error.kind == FrontendErrorKind::Unsupported => {
            return Ok(unsupported_report(
                query,
                "right",
                error,
                config,
                parse_started.elapsed(),
                backend,
            ));
        }
        Err(error) => return Err(error.into()),
    };
    let parse_elapsed = parse_started.elapsed();

    let build_started = Instant::now();
    let left = Nfa::from_expr(&left_expr);
    let right = Nfa::from_expr(&right_expr);
    let build_elapsed = build_started.elapsed();
    let outcome = backend.analyze_binary(query, &left, &right, config);
    let mut report = report_from_binary_outcome(
        query,
        &left,
        &right,
        outcome,
        config,
        parse_elapsed,
        build_elapsed,
        backend,
    );
    validate_binary_witness(&mut report, query, &left, &right)?;
    Ok(report)
}

pub fn analyze_empty(pattern: &str, config: &Config) -> Result<Report, AnalyzeError> {
    analyze_empty_with_backend(pattern, config, &AutomataBackend)
}

pub fn analyze_empty_with_backend(
    pattern: &str,
    config: &Config,
    backend: &dyn RelationBackend,
) -> Result<Report, AnalyzeError> {
    let parse_started = Instant::now();
    let expr = match parse(pattern, config) {
        Ok(expr) => expr,
        Err(error) if error.kind == FrontendErrorKind::Unsupported => {
            return Ok(unsupported_report(
                Query::Empty,
                "regex",
                error,
                config,
                parse_started.elapsed(),
                backend,
            ));
        }
        Err(error) => return Err(error.into()),
    };
    let parse_elapsed = parse_started.elapsed();
    let build_started = Instant::now();
    let nfa = Nfa::from_expr(&expr);
    let build_elapsed = build_started.elapsed();
    let outcome = backend.analyze_empty(&nfa, config);
    let mut report =
        report_from_empty_outcome(&nfa, outcome, config, parse_elapsed, build_elapsed, backend);
    validate_empty_witness(&mut report, &nfa)?;
    Ok(report)
}

fn search_product(query: Query, left: &Nfa, right: &Nfa, config: &Config) -> BackendResult {
    let started = Instant::now();
    let deadline = Duration::from_millis(config.timeout_ms);
    let initial = ProductKey {
        left: left.start_subset(),
        right: right.start_subset(),
    };
    let mut nodes = vec![SearchNode {
        key: initial.clone(),
        parent: None,
    }];
    let mut visited = HashSet::new();
    visited.insert(initial);
    let mut queue = VecDeque::from([0usize]);
    let mut generated_transitions = 0usize;

    while let Some(node_id) = queue.pop_front() {
        if started.elapsed() >= deadline {
            return stopped_result(
                BackendStatus::Timeout,
                nodes.len(),
                generated_transitions,
                started,
            );
        }

        let left_accepts = left.is_accepting(&nodes[node_id].key.left);
        let right_accepts = right.is_accepting(&nodes[node_id].key.right);
        if let Some(relation) = classify_relation(query, left_accepts, right_accepts) {
            let analysis_ms = started.elapsed().as_millis();
            let witness_started = Instant::now();
            let witness = reconstruct(&nodes, node_id);
            let witness_extraction_ms = witness_started.elapsed().as_millis();
            return BackendResult {
                status: BackendStatus::Found,
                witness: Some(witness),
                relation: Some(relation.to_owned()),
                visited_states: nodes.len(),
                generated_transitions,
                analysis_ms,
                witness_extraction_ms,
            };
        }

        let representatives = representative_chars(
            left.outgoing_sets(&nodes[node_id].key.left),
            right.outgoing_sets(&nodes[node_id].key.right),
        );
        for ch in representatives {
            generated_transitions += 1;
            let next = ProductKey {
                left: left.step(&nodes[node_id].key.left, ch),
                right: right.step(&nodes[node_id].key.right, ch),
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

fn search_single(nfa: &Nfa, config: &Config) -> BackendResult {
    let started = Instant::now();
    let deadline = Duration::from_millis(config.timeout_ms);
    let initial = SubsetKey(nfa.start_subset());
    let mut nodes = vec![SearchNode {
        key: initial.clone(),
        parent: None,
    }];
    let mut visited = HashSet::new();
    visited.insert(initial);
    let mut queue = VecDeque::from([0usize]);
    let mut generated_transitions = 0usize;

    while let Some(node_id) = queue.pop_front() {
        if started.elapsed() >= deadline {
            return stopped_result(
                BackendStatus::Timeout,
                nodes.len(),
                generated_transitions,
                started,
            );
        }
        if nfa.is_accepting(&nodes[node_id].key.0) {
            let analysis_ms = started.elapsed().as_millis();
            let witness_started = Instant::now();
            let witness = reconstruct(&nodes, node_id);
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

        let representatives =
            representative_chars(nfa.outgoing_sets(&nodes[node_id].key.0), std::iter::empty());
        for ch in representatives {
            generated_transitions += 1;
            let next = SubsetKey(nfa.step(&nodes[node_id].key.0, ch));
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

fn representative_chars<'a, L, R>(left_sets: L, right_sets: R) -> Vec<char>
where
    L: Iterator<Item = &'a crate::charset::CharSet>,
    R: Iterator<Item = &'a crate::charset::CharSet>,
{
    let sets: Vec<&crate::charset::CharSet> = left_sets.chain(right_sets).collect();
    let mut boundaries = Vec::new();
    for set in &sets {
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

fn reconstruct<K>(nodes: &[SearchNode<K>], mut node_id: usize) -> String {
    let mut reversed = Vec::new();
    while let Some((parent, ch)) = nodes[node_id].parent {
        reversed.push(ch);
        node_id = parent;
    }
    reversed.reverse();
    reversed.into_iter().collect()
}

#[allow(clippy::too_many_arguments)]
fn report_from_binary_outcome(
    query: Query,
    left: &Nfa,
    right: &Nfa,
    outcome: BackendResult,
    config: &Config,
    parse_elapsed: Duration,
    build_elapsed: Duration,
    backend: &dyn RelationBackend,
) -> Report {
    let (verdict, id, message, witness) = match outcome.status {
        BackendStatus::Found => {
            let relation = outcome.relation.as_deref().unwrap_or("counterexample");
            let value = outcome.witness.clone().unwrap_or_default();
            match query {
                Query::Overlap => (
                    Verdict::Yes,
                    "RR_OVERLAP_NONEMPTY",
                    "the two languages overlap",
                    Some(Witness::new(value, relation)),
                ),
                Query::Includes => (
                    Verdict::No,
                    "RR_INCLUDE_COUNTEREXAMPLE",
                    "the left language is not included in the right language",
                    Some(Witness::new(value, relation)),
                ),
                Query::Equivalent => (
                    Verdict::No,
                    "RR_EQUIV_COUNTEREXAMPLE",
                    "the two languages are not equivalent",
                    Some(Witness::new(value, relation)),
                ),
                Query::Empty => unreachable!(),
            }
        }
        BackendStatus::Exhausted => match query {
            Query::Overlap => (
                Verdict::No,
                "RR_OVERLAP_EMPTY",
                "the two languages are disjoint",
                None,
            ),
            Query::Includes => (
                Verdict::Yes,
                "RR_INCLUDED",
                "the left language is included in the right language",
                None,
            ),
            Query::Equivalent => (
                Verdict::Yes,
                "RR_EQUIVALENT",
                "the two languages are equivalent",
                None,
            ),
            Query::Empty => unreachable!(),
        },
        BackendStatus::StateLimit => (
            Verdict::Unknown,
            "RR_STATE_LIMIT",
            "analysis reached the configured product-state limit",
            None,
        ),
        BackendStatus::Timeout => (
            Verdict::Unknown,
            "RR_TIMEOUT",
            "analysis reached the configured timeout",
            None,
        ),
    };

    base_report(
        query,
        verdict,
        witness,
        id,
        message,
        left.states.len(),
        Some(right.states.len()),
        outcome,
        config,
        parse_elapsed,
        build_elapsed,
        backend,
    )
}

fn report_from_empty_outcome(
    nfa: &Nfa,
    outcome: BackendResult,
    config: &Config,
    parse_elapsed: Duration,
    build_elapsed: Duration,
    backend: &dyn RelationBackend,
) -> Report {
    let (verdict, id, message, witness) = match outcome.status {
        BackendStatus::Found => (
            Verdict::No,
            "RR_EMPTY_WITNESS",
            "the language is not empty",
            Some(Witness::new(
                outcome.witness.clone().unwrap_or_default(),
                relation::IN_LANGUAGE,
            )),
        ),
        BackendStatus::Exhausted => (Verdict::Yes, "RR_EMPTY", "the language is empty", None),
        BackendStatus::StateLimit => (
            Verdict::Unknown,
            "RR_STATE_LIMIT",
            "analysis reached the configured state limit",
            None,
        ),
        BackendStatus::Timeout => (
            Verdict::Unknown,
            "RR_TIMEOUT",
            "analysis reached the configured timeout",
            None,
        ),
    };
    base_report(
        Query::Empty,
        verdict,
        witness,
        id,
        message,
        nfa.states.len(),
        None,
        outcome,
        config,
        parse_elapsed,
        build_elapsed,
        backend,
    )
}

#[allow(clippy::too_many_arguments)]
fn base_report(
    query: Query,
    verdict: Verdict,
    witness: Option<Witness>,
    id: &str,
    message: &str,
    left_states: usize,
    right_states: Option<usize>,
    outcome: BackendResult,
    config: &Config,
    parse_elapsed: Duration,
    build_elapsed: Duration,
    backend: &dyn RelationBackend,
) -> Report {
    let mut timings = Timings {
        parsing_ms: parse_elapsed.as_millis(),
        automata_build_ms: build_elapsed.as_millis(),
        backend_ms: outcome.analysis_ms,
        witness_extraction_ms: outcome.witness_extraction_ms,
        ..Timings::default()
    };
    timings.refresh_total();
    Report {
        schema_version: "1",
        tool_version: env!("CARGO_PKG_VERSION"),
        backend: BackendInfo {
            name: backend.name().to_owned(),
            version: backend.version().to_owned(),
        },
        query,
        verdict,
        witness,
        diagnostic: Diagnostic {
            id: id.to_owned(),
            message: message.to_owned(),
            input: None,
            error: None,
            assumptions: assumptions(config),
        },
        statistics: Statistics {
            left_nfa_states: left_states,
            right_nfa_states: right_states,
            visited_product_states: outcome.visited_states,
            generated_transitions: outcome.generated_transitions,
            max_product_states: config.max_product_states,
            timeout_ms: config.timeout_ms,
            alphabet: config.alphabet,
            timings,
        },
        semantics: semantics(config),
    }
}

fn unsupported_report(
    query: Query,
    input: &str,
    error: FrontendError,
    config: &Config,
    parse_elapsed: Duration,
    backend: &dyn RelationBackend,
) -> Report {
    Report {
        schema_version: "1",
        tool_version: env!("CARGO_PKG_VERSION"),
        backend: BackendInfo {
            name: backend.name().to_owned(),
            version: backend.version().to_owned(),
        },
        query,
        verdict: Verdict::Unsupported,
        witness: None,
        diagnostic: Diagnostic {
            id: "RR_UNSUPPORTED".to_owned(),
            message: error.message.clone(),
            input: Some(input.to_owned()),
            error: Some(error),
            assumptions: assumptions(config),
        },
        statistics: Statistics {
            left_nfa_states: 0,
            right_nfa_states: None,
            visited_product_states: 0,
            generated_transitions: 0,
            max_product_states: config.max_product_states,
            timeout_ms: config.timeout_ms,
            alphabet: config.alphabet,
            timings: {
                let mut timings = Timings {
                    parsing_ms: parse_elapsed.as_millis(),
                    ..Timings::default()
                };
                timings.refresh_total();
                timings
            },
        },
        semantics: semantics(config),
    }
}

fn assumptions(config: &Config) -> Vec<String> {
    vec![
        "regexes use full-string matching semantics".to_owned(),
        format!("alphabet mode is {:?}", config.alphabet).to_lowercase(),
        format!("product-state limit is {}", config.max_product_states),
        format!("timeout is {} ms", config.timeout_ms),
        format!("max_repeat is {}", config.max_repeat),
        format!("dot_matches_newline is {}", config.dot_matches_newline),
    ]
}

fn semantics(config: &Config) -> Semantics {
    Semantics {
        match_mode: "full_string",
        witness_order: "shortest_codepoint_length_then_lowest_scalar_value",
        shorthand_classes: "ascii_defined",
        dot_matches_newline: config.dot_matches_newline,
        // Completed verdicts are exact. Resource limits stop with UNKNOWN rather than
        // turning the analysis into a bounded proof.
        bounded: false,
    }
}

fn validate_binary_witness(
    report: &mut Report,
    query: Query,
    left: &Nfa,
    right: &Nfa,
) -> Result<(), AnalyzeError> {
    let Some(witness) = report.witness.as_mut() else {
        return Ok(());
    };
    let started = Instant::now();
    let left_matches = left.matches(&witness.value);
    let right_matches = right.matches(&witness.value);
    let relation = classify_relation(query, left_matches, right_matches);
    let Some(relation) = relation else {
        return Err(AnalyzeError::Internal(format!(
            "witness {:?} replayed as left={left_matches}, right={right_matches}",
            witness.value
        )));
    };
    witness.relation = relation.to_owned();
    report.statistics.timings.witness_validation_ms = started.elapsed().as_millis();
    report.statistics.timings.refresh_total();
    Ok(())
}

fn validate_empty_witness(report: &mut Report, nfa: &Nfa) -> Result<(), AnalyzeError> {
    let Some(witness) = report.witness.as_mut() else {
        return Ok(());
    };
    let started = Instant::now();
    if !nfa.matches(&witness.value) {
        return Err(AnalyzeError::Internal(format!(
            "witness {:?} did not replay in the language",
            witness.value
        )));
    }
    witness.relation = relation::IN_LANGUAGE.to_owned();
    report.statistics.timings.witness_validation_ms = started.elapsed().as_millis();
    report.statistics.timings.refresh_total();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_witness_is_shortest() {
        let report = analyze_binary(Query::Overlap, "a+b", "ab+", &Config::default()).unwrap();
        assert_eq!(report.verdict, Verdict::Yes);
        assert_eq!(report.witness.unwrap().value, "ab");
    }

    #[test]
    fn inclusion_counterexample() {
        let report =
            analyze_binary(Query::Includes, "[a-z]+", "[a-z]{2,}", &Config::default()).unwrap();
        assert_eq!(report.verdict, Verdict::No);
        assert_eq!(report.witness.unwrap().value, "a");
    }

    #[test]
    fn equivalence_example() {
        let report = analyze_binary(Query::Equivalent, "a|b", "[ab]", &Config::default()).unwrap();
        assert_eq!(report.verdict, Verdict::Yes);
    }

    #[test]
    fn equivalence_witness_records_its_direction() {
        let left_only = analyze_binary(Query::Equivalent, "a|b", "b", &Config::default()).unwrap();
        assert_eq!(left_only.verdict, Verdict::No);
        assert_eq!(left_only.witness.unwrap().relation, relation::LEFT_ONLY);

        let right_only = analyze_binary(Query::Equivalent, "b", "a|b", &Config::default()).unwrap();
        assert_eq!(right_only.verdict, Verdict::No);
        let witness = right_only.witness.unwrap();
        assert_eq!(witness.value, "a");
        assert_eq!(witness.relation, relation::RIGHT_ONLY);
    }

    #[test]
    fn detects_an_empty_character_class_language() {
        let report = analyze_empty(r"[^\d\D]", &Config::default()).unwrap();
        assert_eq!(report.verdict, Verdict::Yes);
        assert!(report.witness.is_none());
    }

    #[test]
    fn empty_regex_language_is_not_empty() {
        let report = analyze_empty("", &Config::default()).unwrap();
        assert_eq!(report.verdict, Verdict::No);
        assert_eq!(report.witness.unwrap().value, "");
    }

    #[test]
    fn deterministic_tie_break_uses_lowest_scalar() {
        let report = analyze_binary(Query::Overlap, "[ba]", "[ab]", &Config::default()).unwrap();
        assert_eq!(report.verdict, Verdict::Yes);
        assert_eq!(report.witness.unwrap().value, "a");
    }

    #[test]
    fn empty_string_is_preferred_over_nonempty_witnesses() {
        let report = analyze_binary(Query::Overlap, "a*", "b*", &Config::default()).unwrap();
        assert_eq!(report.verdict, Verdict::Yes);
        assert_eq!(report.witness.unwrap().value, "");
    }

    #[test]
    fn witness_codepoint_count_handles_unicode() {
        let config = Config {
            alphabet: crate::config::Alphabet::Unicode,
            ..Config::default()
        };
        let report = analyze_binary(Query::Overlap, "é+", "éé*", &config).unwrap();
        let witness = report.witness.unwrap();
        assert_eq!(witness.value, "é");
        assert_eq!(witness.codepoints, 1);
    }

    #[test]
    fn dot_newline_configuration_changes_the_relation() {
        let default_report =
            analyze_binary(Query::Overlap, ".", r"\n", &Config::default()).unwrap();
        assert_eq!(default_report.verdict, Verdict::No);

        let config = Config {
            dot_matches_newline: true,
            ..Config::default()
        };
        let configured = analyze_binary(Query::Overlap, ".", r"\n", &config).unwrap();
        assert_eq!(configured.verdict, Verdict::Yes);
        assert_eq!(configured.witness.unwrap().value, "\n");
    }

    fn finite_words(alphabet: &[char], max_len: usize) -> Vec<String> {
        let mut words = vec![String::new()];
        let mut frontier = vec![String::new()];
        for _ in 0..max_len {
            let mut next = Vec::new();
            for prefix in &frontier {
                for ch in alphabet {
                    let mut word = prefix.clone();
                    word.push(*ch);
                    words.push(word.clone());
                    next.push(word);
                }
            }
            frontier = next;
        }
        words
    }

    fn concrete_verdict(query: Query, left: &Nfa, right: &Nfa, words: &[String]) -> Verdict {
        let property_holds = match query {
            Query::Overlap => words
                .iter()
                .any(|word| left.matches(word) && right.matches(word)),
            Query::Includes => words
                .iter()
                .all(|word| !left.matches(word) || right.matches(word)),
            Query::Equivalent => words
                .iter()
                .all(|word| left.matches(word) == right.matches(word)),
            Query::Empty => unreachable!(),
        };
        if property_holds {
            Verdict::Yes
        } else {
            Verdict::No
        }
    }

    #[test]
    fn all_binary_queries_agree_with_concrete_execution_on_finite_languages() {
        let patterns = ["", "a", "b", "a|b", "ab", "a?", "[ab]", "[ab]{2}", "a{0,2}"];
        let words = finite_words(&['a', 'b'], 2);
        let config = Config::default();

        for left_pattern in patterns {
            for right_pattern in patterns {
                let left = Nfa::from_expr(&parse(left_pattern, &config).unwrap());
                let right = Nfa::from_expr(&parse(right_pattern, &config).unwrap());
                for query in [Query::Overlap, Query::Includes, Query::Equivalent] {
                    let expected = concrete_verdict(query, &left, &right, &words);
                    let report =
                        analyze_binary(query, left_pattern, right_pattern, &config).unwrap();
                    assert_eq!(
                        report.verdict, expected,
                        "{query:?}: {left_pattern:?}, {right_pattern:?}"
                    );
                    if let Some(witness) = report.witness {
                        let left_matches = left.matches(&witness.value);
                        let right_matches = right.matches(&witness.value);
                        assert!(match query {
                            Query::Overlap => left_matches && right_matches,
                            Query::Includes => left_matches && !right_matches,
                            Query::Equivalent => left_matches != right_matches,
                            Query::Empty => unreachable!(),
                        });
                    }
                }
            }
        }
    }

    #[test]
    fn equivalence_is_reflexive_on_small_corpus() {
        let patterns = ["", "a", "a*", "a|b", "[ab]+", "(ab){1,2}"];
        for pattern in patterns {
            let report =
                analyze_binary(Query::Equivalent, pattern, pattern, &Config::default()).unwrap();
            assert_eq!(report.verdict, Verdict::Yes, "{pattern:?}");
        }
    }

    #[test]
    fn inclusion_is_reflexive_on_small_corpus() {
        let patterns = ["", "a", "a*", "a|b", "[ab]+", "(ab){1,2}"];
        for pattern in patterns {
            let report =
                analyze_binary(Query::Includes, pattern, pattern, &Config::default()).unwrap();
            assert_eq!(report.verdict, Verdict::Yes, "{pattern:?}");
        }
    }

    #[test]
    fn overlap_is_symmetric_on_small_corpus() {
        let patterns = ["", "a", "b", "a*", "ab", "a|b", "[ab]+"];
        for left in patterns {
            for right in patterns {
                let forward =
                    analyze_binary(Query::Overlap, left, right, &Config::default()).unwrap();
                let backward =
                    analyze_binary(Query::Overlap, right, left, &Config::default()).unwrap();
                assert_eq!(forward.verdict, backward.verdict);
                assert_eq!(
                    forward.witness.as_ref().map(|witness| &witness.value),
                    backward.witness.as_ref().map(|witness| &witness.value),
                );
            }
        }
    }

    #[test]
    fn equivalence_matches_mutual_inclusion() {
        let patterns = ["", "a", "b", "a?", "a|b", "[ab]", "[ab]{2}"];
        for left in patterns {
            for right in patterns {
                let equivalent =
                    analyze_binary(Query::Equivalent, left, right, &Config::default()).unwrap();
                let forward =
                    analyze_binary(Query::Includes, left, right, &Config::default()).unwrap();
                let backward =
                    analyze_binary(Query::Includes, right, left, &Config::default()).unwrap();
                assert_eq!(
                    equivalent.verdict == Verdict::Yes,
                    forward.verdict == Verdict::Yes && backward.verdict == Verdict::Yes,
                    "{left:?}, {right:?}"
                );
            }
        }
    }

    #[test]
    fn inclusion_is_transitive_for_a_known_chain() {
        let ab_in_word =
            analyze_binary(Query::Includes, "[ab]+", r"\w+", &Config::default()).unwrap();
        let word_in_any =
            analyze_binary(Query::Includes, r"\w+", ".+", &Config::default()).unwrap();
        let ab_in_any = analyze_binary(Query::Includes, "[ab]+", ".+", &Config::default()).unwrap();
        assert_eq!(ab_in_word.verdict, Verdict::Yes);
        assert_eq!(word_in_any.verdict, Verdict::Yes);
        assert_eq!(ab_in_any.verdict, Verdict::Yes);
    }

    #[test]
    fn unsupported_input_is_a_first_class_report() {
        let report = analyze_binary(Query::Equivalent, r"(a)\1", "a", &Config::default()).unwrap();
        assert_eq!(report.verdict, Verdict::Unsupported);
        assert_eq!(report.diagnostic.id, "RR_UNSUPPORTED");
        assert_eq!(report.diagnostic.input.as_deref(), Some("left"));
        assert!(report.diagnostic.error.is_some());
    }

    struct FakeBackend;

    impl RelationBackend for FakeBackend {
        fn name(&self) -> &'static str {
            "fake"
        }

        fn version(&self) -> &'static str {
            "test"
        }

        fn analyze_binary(
            &self,
            _query: Query,
            _left: &Nfa,
            _right: &Nfa,
            _config: &Config,
        ) -> BackendResult {
            BackendResult {
                status: BackendStatus::StateLimit,
                witness: None,
                relation: None,
                visited_states: 7,
                generated_transitions: 11,
                analysis_ms: 0,
                witness_extraction_ms: 0,
            }
        }

        fn analyze_empty(&self, _nfa: &Nfa, _config: &Config) -> BackendResult {
            BackendResult {
                status: BackendStatus::Exhausted,
                witness: None,
                relation: None,
                visited_states: 1,
                generated_transitions: 0,
                analysis_ms: 0,
                witness_extraction_ms: 0,
            }
        }
    }

    #[test]
    fn custom_backend_is_reported() {
        let report = analyze_binary_with_backend(
            Query::Equivalent,
            "a",
            "a",
            &Config::default(),
            &FakeBackend,
        )
        .unwrap();
        assert_eq!(report.verdict, Verdict::Unknown);
        assert_eq!(report.backend.name, "fake");
        assert_eq!(report.statistics.visited_product_states, 7);
    }

    struct TimeoutBackend;

    impl RelationBackend for TimeoutBackend {
        fn name(&self) -> &'static str {
            "timeout_fake"
        }

        fn version(&self) -> &'static str {
            "test"
        }

        fn analyze_binary(
            &self,
            _query: Query,
            _left: &Nfa,
            _right: &Nfa,
            _config: &Config,
        ) -> BackendResult {
            BackendResult {
                status: BackendStatus::Timeout,
                witness: None,
                relation: None,
                visited_states: 3,
                generated_transitions: 4,
                analysis_ms: 5,
                witness_extraction_ms: 0,
            }
        }

        fn analyze_empty(&self, _nfa: &Nfa, _config: &Config) -> BackendResult {
            BackendResult {
                status: BackendStatus::Timeout,
                witness: None,
                relation: None,
                visited_states: 3,
                generated_transitions: 4,
                analysis_ms: 5,
                witness_extraction_ms: 0,
            }
        }
    }

    #[test]
    fn timeout_backend_yields_unknown_with_stable_diagnostic() {
        let report = analyze_binary_with_backend(
            Query::Overlap,
            "a",
            "a",
            &Config::default(),
            &TimeoutBackend,
        )
        .unwrap();
        assert_eq!(report.verdict, Verdict::Unknown);
        assert_eq!(report.diagnostic.id, "RR_TIMEOUT");
        assert_eq!(report.statistics.visited_product_states, 3);
    }

    struct TimedWitnessBackend;

    impl RelationBackend for TimedWitnessBackend {
        fn name(&self) -> &'static str {
            "timed_witness_fake"
        }

        fn version(&self) -> &'static str {
            "test"
        }

        fn analyze_binary(
            &self,
            _query: Query,
            _left: &Nfa,
            _right: &Nfa,
            _config: &Config,
        ) -> BackendResult {
            BackendResult {
                status: BackendStatus::Found,
                witness: Some("a".to_owned()),
                relation: Some(relation::IN_BOTH.to_owned()),
                visited_states: 1,
                generated_transitions: 0,
                analysis_ms: 5,
                witness_extraction_ms: 7,
            }
        }

        fn analyze_empty(&self, _nfa: &Nfa, _config: &Config) -> BackendResult {
            unreachable!()
        }
    }

    #[test]
    fn backend_and_witness_extraction_timings_are_separate_components() {
        let report = analyze_binary_with_backend(
            Query::Overlap,
            "a",
            "a",
            &Config::default(),
            &TimedWitnessBackend,
        )
        .unwrap();
        let timings = report.statistics.timings;
        assert_eq!(timings.backend_ms, 5);
        assert_eq!(timings.witness_extraction_ms, 7);
        assert_eq!(
            timings.total_ms,
            timings.parsing_ms
                + timings.automata_build_ms
                + timings.backend_ms
                + timings.witness_extraction_ms
                + timings.witness_validation_ms
                + timings.rendering_ms
        );
    }

    struct InvalidWitnessBackend;

    impl RelationBackend for InvalidWitnessBackend {
        fn name(&self) -> &'static str {
            "invalid_witness_fake"
        }

        fn version(&self) -> &'static str {
            "test"
        }

        fn analyze_binary(
            &self,
            _query: Query,
            _left: &Nfa,
            _right: &Nfa,
            _config: &Config,
        ) -> BackendResult {
            BackendResult {
                status: BackendStatus::Found,
                witness: Some("b".to_owned()),
                relation: Some(relation::IN_BOTH.to_owned()),
                visited_states: 1,
                generated_transitions: 0,
                analysis_ms: 0,
                witness_extraction_ms: 0,
            }
        }

        fn analyze_empty(&self, _nfa: &Nfa, _config: &Config) -> BackendResult {
            unreachable!()
        }
    }

    #[test]
    fn invalid_backend_witness_is_rejected_as_internal_error() {
        let error = analyze_binary_with_backend(
            Query::Overlap,
            "a",
            "a",
            &Config::default(),
            &InvalidWitnessBackend,
        )
        .unwrap_err();
        assert!(matches!(error, AnalyzeError::Internal(_)));
    }

    #[test]
    fn state_limit_yields_unknown() {
        let config = Config {
            max_product_states: 1,
            ..Config::default()
        };
        let report = analyze_binary(Query::Equivalent, "a", "b", &config).unwrap();
        assert_eq!(report.verdict, Verdict::Unknown);
        assert_eq!(report.diagnostic.id, "RR_STATE_LIMIT");
    }

    #[test]
    fn initial_witness_is_found_even_at_minimum_state_limit() {
        let config = Config {
            max_product_states: 1,
            ..Config::default()
        };
        let report = analyze_binary(Query::Overlap, "a*", "b*", &config).unwrap();
        assert_eq!(report.verdict, Verdict::Yes);
        assert_eq!(report.witness.unwrap().value, "");
    }

    #[test]
    fn empty_query_can_also_return_unknown_at_state_limit() {
        let config = Config {
            max_product_states: 1,
            ..Config::default()
        };
        let report = analyze_empty("a", &config).unwrap();
        assert_eq!(report.verdict, Verdict::Unknown);
        assert_eq!(report.diagnostic.id, "RR_STATE_LIMIT");
    }
}
