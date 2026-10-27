//! Complementary backend portfolio with synchronizing-separator decomposition.
//!
//! The existing engines have deliberately different failure modes.  In the
//! experiment matrix, adaptive automata solves the suffix/window family while
//! forward HKC solves the symmetric `event_two_windows_*` family that remains
//! exponential for subset products.  This module combines those strengths
//! without changing the historical `automata` backend.
//!
//! Before spending budget on either general solver, binary inclusion and
//! equivalence queries look for a *provably unique shared separator*: a
//! top-level singleton character that occurs exactly once in every word from
//! both expressions because no other top-level component can consume it.  For
//! languages `A # B` and `C # D`, uniqueness makes the split injective, so
//! inclusion decomposes into `A ⊆ C` and `B ⊆ D` (unless the source product is
//! empty), and equivalence is the two directional inclusions.  Counterexamples
//! are composed from canonical segment counterexamples plus canonical shortest
//! members, then compared globally by the public shortlex witness order. The
//! separator proof needs the parsed AST; the trait's NFA-only `analyze_binary`
//! entry point therefore uses the general portfolio fallback directly.
//!
//! If decomposition is unavailable or its bounded attempt cannot decide, the
//! remaining shared state/time budget is used by an automata-first, HKC-second
//! portfolio.  No attempt gets a fresh copy of the caller budget.

use crate::analysis::{
    AutomataBackend, BackendResult, BackendStatus, Query, RelationBackend,
};
use crate::ast::{Expr, ExprKind, Span};
use crate::config::Config;
use crate::hkc::HkcBackend;
use crate::nfa::Nfa;
use crate::report::relation;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::time::{Duration, Instant};

const SEPARATOR_MAX_STATES: usize = 8_192;
const SEPARATOR_MAX_MS: u64 = 250;
const SEGMENT_FORWARD_MAX_STATES: usize = 512;
const SEGMENT_FORWARD_MAX_MS: u64 = 25;
const SEGMENT_REVERSE_MAX_STATES: usize = 2_048;
const SEGMENT_REVERSE_MAX_MS: u64 = 100;
const PRIMARY_NUMERATOR: usize = 2;
const PRIMARY_DENOMINATOR: usize = 3;

#[derive(Clone, Copy, Debug, Default)]
pub struct PortfolioBackend;

impl RelationBackend for PortfolioBackend {
    fn name(&self) -> &'static str {
        "portfolio"
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
        run_general_portfolio(query, left, right, config, Work::default(), Instant::now())
    }

    fn analyze_binary_expr(
        &self,
        query: Query,
        left_expr: &Expr,
        right_expr: &Expr,
        left: &Nfa,
        right: &Nfa,
        config: &Config,
    ) -> BackendResult {
        let started = Instant::now();
        let separator_attempt = if matches!(query, Query::Includes | Query::Equivalent) {
            try_separator_decomposition(query, left_expr, right_expr, config, started)
        } else {
            SeparatorAttempt::not_applicable()
        };
        if let Some(result) = separator_attempt.result {
            return result;
        }
        run_general_portfolio(
            query,
            left,
            right,
            config,
            separator_attempt.work,
            started,
        )
    }

    fn analyze_empty(&self, nfa: &Nfa, config: &Config) -> BackendResult {
        AutomataBackend.analyze_empty(nfa, config)
    }
}

#[derive(Clone, Debug)]
struct Segment {
    expr: Expr,
    nfa: Nfa,
}

impl Segment {
    fn new(expr: Expr) -> Self {
        let nfa = Nfa::from_expr(&expr);
        Self { expr, nfa }
    }
}

#[derive(Clone, Debug)]
struct Split {
    separator: char,
    prefix: Segment,
    suffix: Segment,
}

#[derive(Clone, Copy, Debug, Default)]
struct Work {
    visited_states: usize,
    generated_transitions: usize,
    witness_extraction_ms: u128,
}

impl Work {
    fn absorb(&mut self, result: &BackendResult) {
        self.visited_states = self.visited_states.saturating_add(result.visited_states);
        self.generated_transitions = self
            .generated_transitions
            .saturating_add(result.generated_transitions);
        self.witness_extraction_ms = self
            .witness_extraction_ms
            .saturating_add(result.witness_extraction_ms);
    }
}

struct SeparatorAttempt {
    result: Option<BackendResult>,
    work: Work,
}

impl SeparatorAttempt {
    fn not_applicable() -> Self {
        Self {
            result: None,
            work: Work::default(),
        }
    }
}

struct DecompositionRunner<'a> {
    base: &'a Config,
    started: Instant,
    timeout_ms: u64,
    state_budget: usize,
    work: Work,
}

impl<'a> DecompositionRunner<'a> {
    fn new(base: &'a Config, started: Instant) -> Self {
        Self {
            base,
            started,
            timeout_ms: base.timeout_ms.clamp(1, SEPARATOR_MAX_MS),
            state_budget: base.max_product_states.clamp(1, SEPARATOR_MAX_STATES),
            work: Work::default(),
        }
    }

    fn remaining_config(&self) -> Option<Config> {
        let elapsed = self.started.elapsed().as_millis();
        if elapsed >= u128::from(self.timeout_ms) || self.work.visited_states >= self.state_budget {
            return None;
        }
        let mut config = self.base.clone();
        config.max_product_states = self.state_budget - self.work.visited_states;
        config.timeout_ms = (u128::from(self.timeout_ms) - elapsed)
            .min(u128::from(u64::MAX)) as u64;
        config.timeout_ms = config.timeout_ms.max(1);
        Some(config)
    }

    fn absorb(&mut self, result: &BackendResult) {
        self.work.absorb(result);
    }

    fn shortest_member(&mut self, segment: &Segment) -> Option<Option<String>> {
        let config = self.remaining_config()?;
        let result = shortest_nfa_word(&segment.nfa, &config);
        self.absorb(&result);
        match result.status {
            BackendStatus::Found => Some(result.witness),
            BackendStatus::Exhausted => Some(None),
            BackendStatus::StateLimit | BackendStatus::Timeout | BackendStatus::Unsupported => None,
        }
    }

    fn inclusion(&mut self, left: &Segment, right: &Segment) -> Option<Option<String>> {
        if left.expr == right.expr {
            return Some(None);
        }

        let remaining = self.remaining_config()?;
        let mut forward_config = remaining.clone();
        forward_config.max_product_states = remaining
            .max_product_states
            .div_ceil(4)
            .clamp(1, SEGMENT_FORWARD_MAX_STATES);
        forward_config.timeout_ms = remaining
            .timeout_ms
            .div_ceil(4)
            .clamp(1, SEGMENT_FORWARD_MAX_MS);
        let forward = AutomataBackend.analyze_binary(
            Query::Includes,
            &left.nfa,
            &right.nfa,
            &forward_config,
        );
        self.absorb(&forward);
        match forward.status {
            BackendStatus::Exhausted => return Some(None),
            BackendStatus::Found => return Some(forward.witness),
            BackendStatus::StateLimit | BackendStatus::Timeout => {}
            BackendStatus::Unsupported => return None,
        }

        let remaining = self.remaining_config()?;
        let mut reverse_config = remaining.clone();
        reverse_config.max_product_states = remaining
            .max_product_states
            .clamp(1, SEGMENT_REVERSE_MAX_STATES);
        reverse_config.timeout_ms = remaining.timeout_ms.clamp(1, SEGMENT_REVERSE_MAX_MS);
        let reversed = AutomataBackend
            .analyze_binary_reversed_nfa(
                Query::Includes,
                &left.nfa,
                &right.nfa,
                &reverse_config,
            )
            .expect("automata implements exact reversed inclusion");
        self.absorb(&reversed);
        match reversed.status {
            BackendStatus::Exhausted => Some(None),
            BackendStatus::Found => Some(reversed.witness),
            BackendStatus::StateLimit | BackendStatus::Timeout | BackendStatus::Unsupported => None,
        }
    }

    fn finish(
        &self,
        status: BackendStatus,
        witness: Option<String>,
        relation: Option<&str>,
    ) -> BackendResult {
        let witness_extraction_ms = self.work.witness_extraction_ms;
        BackendResult {
            status,
            witness,
            relation: relation.map(str::to_owned),
            visited_states: self.work.visited_states,
            generated_transitions: self.work.generated_transitions,
            analysis_ms: self
                .started
                .elapsed()
                .as_millis()
                .saturating_sub(witness_extraction_ms),
            witness_extraction_ms,
        }
    }
}

fn try_separator_decomposition(
    query: Query,
    left_expr: &Expr,
    right_expr: &Expr,
    config: &Config,
    started: Instant,
) -> SeparatorAttempt {
    let Some((left, right)) = common_unique_separator(left_expr, right_expr) else {
        return SeparatorAttempt::not_applicable();
    };
    debug_assert_eq!(left.separator, right.separator);
    let separator = left.separator;
    let mut runner = DecompositionRunner::new(config, started);

    let result = match query {
        Query::Includes => decomposed_inclusion(
            &mut runner,
            &left,
            &right,
            separator,
            relation::LEFT_ONLY,
        ),
        Query::Equivalent => decomposed_equivalence(&mut runner, &left, &right, separator),
        Query::Overlap | Query::Empty | Query::Match => None,
    };

    SeparatorAttempt {
        result,
        work: runner.work,
    }
}

fn decomposed_inclusion(
    runner: &mut DecompositionRunner<'_>,
    left: &Split,
    right: &Split,
    separator: char,
    relation_name: &str,
) -> Option<BackendResult> {
    let left_prefix_min = runner.shortest_member(&left.prefix)?;
    let left_suffix_min = runner.shortest_member(&left.suffix)?;
    let (Some(prefix_min), Some(suffix_min)) = (left_prefix_min, left_suffix_min) else {
        return Some(runner.finish(BackendStatus::Exhausted, None, None));
    };

    let prefix_violation = runner.inclusion(&left.prefix, &right.prefix)?;
    let suffix_violation = runner.inclusion(&left.suffix, &right.suffix)?;

    let mut candidates = Vec::new();
    if let Some(prefix_bad) = prefix_violation {
        candidates.push(join_word(&prefix_bad, separator, &suffix_min));
    }
    if let Some(suffix_bad) = suffix_violation {
        candidates.push(join_word(&prefix_min, separator, &suffix_bad));
    }

    if let Some(witness) = candidates.into_iter().min_by(|a, b| shortlex_cmp(a, b)) {
        Some(runner.finish(
            BackendStatus::Found,
            Some(witness),
            Some(relation_name),
        ))
    } else {
        Some(runner.finish(BackendStatus::Exhausted, None, None))
    }
}

fn decomposed_equivalence(
    runner: &mut DecompositionRunner<'_>,
    left: &Split,
    right: &Split,
    separator: char,
) -> Option<BackendResult> {
    let left_prefix_min = runner.shortest_member(&left.prefix)?;
    let left_suffix_min = runner.shortest_member(&left.suffix)?;
    let right_prefix_min = runner.shortest_member(&right.prefix)?;
    let right_suffix_min = runner.shortest_member(&right.suffix)?;

    let left_nonempty = left_prefix_min.is_some() && left_suffix_min.is_some();
    let right_nonempty = right_prefix_min.is_some() && right_suffix_min.is_some();
    match (left_nonempty, right_nonempty) {
        (false, false) => return Some(runner.finish(BackendStatus::Exhausted, None, None)),
        (true, false) => {
            let witness = join_word(
                left_prefix_min.as_deref().expect("checked nonempty"),
                separator,
                left_suffix_min.as_deref().expect("checked nonempty"),
            );
            return Some(runner.finish(
                BackendStatus::Found,
                Some(witness),
                Some(relation::LEFT_ONLY),
            ));
        }
        (false, true) => {
            let witness = join_word(
                right_prefix_min.as_deref().expect("checked nonempty"),
                separator,
                right_suffix_min.as_deref().expect("checked nonempty"),
            );
            return Some(runner.finish(
                BackendStatus::Found,
                Some(witness),
                Some(relation::RIGHT_ONLY),
            ));
        }
        (true, true) => {}
    }

    let left_prefix_min = left_prefix_min.expect("checked nonempty");
    let left_suffix_min = left_suffix_min.expect("checked nonempty");
    let right_prefix_min = right_prefix_min.expect("checked nonempty");
    let right_suffix_min = right_suffix_min.expect("checked nonempty");

    let mut candidates: Vec<(String, &'static str)> = Vec::new();
    if let Some(prefix_bad) = runner.inclusion(&left.prefix, &right.prefix)? {
        candidates.push((
            join_word(&prefix_bad, separator, &left_suffix_min),
            relation::LEFT_ONLY,
        ));
    }
    if let Some(suffix_bad) = runner.inclusion(&left.suffix, &right.suffix)? {
        candidates.push((
            join_word(&left_prefix_min, separator, &suffix_bad),
            relation::LEFT_ONLY,
        ));
    }
    if let Some(prefix_bad) = runner.inclusion(&right.prefix, &left.prefix)? {
        candidates.push((
            join_word(&prefix_bad, separator, &right_suffix_min),
            relation::RIGHT_ONLY,
        ));
    }
    if let Some(suffix_bad) = runner.inclusion(&right.suffix, &left.suffix)? {
        candidates.push((
            join_word(&right_prefix_min, separator, &suffix_bad),
            relation::RIGHT_ONLY,
        ));
    }

    if let Some((witness, relation_name)) = candidates
        .into_iter()
        .min_by(|(a, _), (b, _)| shortlex_cmp(a, b))
    {
        Some(runner.finish(
            BackendStatus::Found,
            Some(witness),
            Some(relation_name),
        ))
    } else {
        Some(runner.finish(BackendStatus::Exhausted, None, None))
    }
}

fn common_unique_separator(left: &Expr, right: &Expr) -> Option<(Split, Split)> {
    if contains_anchor(left) || contains_anchor(right) {
        return None;
    }
    let mut candidates = separator_candidates(left);
    candidates.sort_unstable();
    candidates.dedup();
    for separator in candidates {
        let Some(left_split) = split_for_separator(left, separator) else {
            continue;
        };
        let Some(right_split) = split_for_separator(right, separator) else {
            continue;
        };
        return Some((left_split, right_split));
    }
    None
}

fn separator_candidates(expr: &Expr) -> Vec<char> {
    top_level_parts(expr)
        .iter()
        .filter_map(|part| singleton_atom(part))
        .collect()
}

fn split_for_separator(expr: &Expr, separator: char) -> Option<Split> {
    let parts = top_level_parts(expr);
    let mut separator_index = None;
    for (index, part) in parts.iter().enumerate() {
        if singleton_atom(part) == Some(separator) {
            if separator_index.replace(index).is_some() {
                return None;
            }
        } else if can_consume_char(part, separator) {
            return None;
        }
    }
    let index = separator_index?;
    let separator_span = parts[index].span;
    let prefix = concat_parts(
        &parts[..index],
        Span::new(separator_span.start, separator_span.start),
    );
    let suffix = concat_parts(
        &parts[index + 1..],
        Span::new(separator_span.end, separator_span.end),
    );
    Some(Split {
        separator,
        prefix: Segment::new(prefix),
        suffix: Segment::new(suffix),
    })
}

fn top_level_parts(expr: &Expr) -> Vec<Expr> {
    match &expr.kind {
        ExprKind::Concat(parts) => parts.clone(),
        _ => vec![expr.clone()],
    }
}

fn concat_parts(parts: &[Expr], empty_span: Span) -> Expr {
    match parts {
        [] => Expr::new(ExprKind::Empty, empty_span),
        [only] => only.clone(),
        _ => Expr::new(
            ExprKind::Concat(parts.to_vec()),
            parts
                .first()
                .expect("nonempty")
                .span
                .join(parts.last().expect("nonempty").span),
        ),
    }
}

fn singleton_atom(expr: &Expr) -> Option<char> {
    match &expr.kind {
        ExprKind::Literal(ch) => Some(*ch),
        ExprKind::CharSet(set) => set.as_singleton(),
        _ => None,
    }
}

fn can_consume_char(expr: &Expr, ch: char) -> bool {
    match &expr.kind {
        ExprKind::Empty | ExprKind::AnchorStart | ExprKind::AnchorEnd => false,
        ExprKind::Literal(literal) => *literal == ch,
        ExprKind::CharSet(set) => set.contains(ch),
        ExprKind::Concat(parts) | ExprKind::Alt(parts) => {
            parts.iter().any(|part| can_consume_char(part, ch))
        }
        ExprKind::Repeat { expr, max, .. } => {
            *max != Some(0) && can_consume_char(expr, ch)
        }
    }
}

fn contains_anchor(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::AnchorStart | ExprKind::AnchorEnd => true,
        ExprKind::Concat(parts) | ExprKind::Alt(parts) => parts.iter().any(contains_anchor),
        ExprKind::Repeat { expr, .. } => contains_anchor(expr),
        ExprKind::Empty | ExprKind::Literal(_) | ExprKind::CharSet(_) => false,
    }
}

fn join_word(prefix: &str, separator: char, suffix: &str) -> String {
    let mut result = String::with_capacity(prefix.len() + separator.len_utf8() + suffix.len());
    result.push_str(prefix);
    result.push(separator);
    result.push_str(suffix);
    result
}

fn shortlex_cmp(left: &str, right: &str) -> Ordering {
    left.chars()
        .count()
        .cmp(&right.chars().count())
        .then_with(|| left.chars().cmp(right.chars()))
}

#[derive(Clone, Eq, PartialEq)]
struct PathItem {
    length: usize,
    word: String,
    state: usize,
}

impl Ord for PathItem {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .length
            .cmp(&self.length)
            .then_with(|| other.word.cmp(&self.word))
            .then_with(|| other.state.cmp(&self.state))
    }
}

impl PartialOrd for PathItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn shortest_nfa_word(nfa: &Nfa, config: &Config) -> BackendResult {
    let started = Instant::now();
    let deadline = Duration::from_millis(config.timeout_ms);
    let mut heap = BinaryHeap::new();
    let mut best: Vec<Option<(usize, String)>> = vec![None; nfa.states.len()];
    let mut settled = vec![false; nfa.states.len()];
    let mut visited_states = 0usize;
    let mut generated_transitions = 0usize;

    best[nfa.start] = Some((0, String::new()));
    heap.push(PathItem {
        length: 0,
        word: String::new(),
        state: nfa.start,
    });

    while let Some(item) = heap.pop() {
        if started.elapsed() >= deadline {
            return raw_result(
                BackendStatus::Timeout,
                None,
                None,
                visited_states,
                generated_transitions,
                started,
                0,
            );
        }
        if settled[item.state] {
            continue;
        }
        let Some((best_length, best_word)) = &best[item.state] else {
            continue;
        };
        if *best_length != item.length || best_word != &item.word {
            continue;
        }
        if visited_states >= config.max_product_states {
            return raw_result(
                BackendStatus::StateLimit,
                None,
                None,
                visited_states,
                generated_transitions,
                started,
                0,
            );
        }
        settled[item.state] = true;
        visited_states += 1;

        if item.state == nfa.accept {
            return raw_result(
                BackendStatus::Found,
                Some(item.word),
                Some(relation::IN_LANGUAGE),
                visited_states,
                generated_transitions,
                started,
                0,
            );
        }

        let state = &nfa.states[item.state];
        for &target in &state.epsilon {
            generated_transitions = generated_transitions.saturating_add(1);
            relax_path(&mut best, &mut heap, target, item.length, item.word.clone());
        }
        for transition in &state.transitions {
            generated_transitions = generated_transitions.saturating_add(1);
            let Some(ch) = transition
                .set
                .intervals()
                .first()
                .and_then(|interval| char::from_u32(interval.start))
            else {
                continue;
            };
            let mut word = item.word.clone();
            word.push(ch);
            relax_path(
                &mut best,
                &mut heap,
                transition.target,
                item.length.saturating_add(1),
                word,
            );
        }
    }

    raw_result(
        BackendStatus::Exhausted,
        None,
        None,
        visited_states,
        generated_transitions,
        started,
        0,
    )
}

fn relax_path(
    best: &mut [Option<(usize, String)>],
    heap: &mut BinaryHeap<PathItem>,
    state: usize,
    length: usize,
    word: String,
) {
    let improve = match &best[state] {
        None => true,
        Some((old_length, old_word)) => {
            length < *old_length
                || (length == *old_length && word.chars().cmp(old_word.chars()).is_lt())
        }
    };
    if improve {
        best[state] = Some((length, word.clone()));
        heap.push(PathItem {
            length,
            word,
            state,
        });
    }
}

fn run_general_portfolio(
    query: Query,
    left: &Nfa,
    right: &Nfa,
    config: &Config,
    mut prior_work: Work,
    started: Instant,
) -> BackendResult {
    let Some(remaining) = remaining_global_config(config, started, prior_work.visited_states) else {
        return budget_exhausted_result(config, prior_work, started);
    };

    if query == Query::Overlap {
        let mut result = AutomataBackend.analyze_binary(query, left, right, &remaining);
        merge_work(&mut result, prior_work, started);
        return result;
    }

    let mut primary_config = remaining.clone();
    primary_config.max_product_states = remaining
        .max_product_states
        .saturating_mul(PRIMARY_NUMERATOR)
        .div_ceil(PRIMARY_DENOMINATOR)
        .max(1);
    primary_config.timeout_ms = remaining
        .timeout_ms
        .saturating_mul(PRIMARY_NUMERATOR as u64)
        .div_ceil(PRIMARY_DENOMINATOR as u64)
        .max(1);

    let primary = AutomataBackend.analyze_binary(query, left, right, &primary_config);
    if is_definitive(primary.status) {
        let mut result = primary;
        merge_work(&mut result, prior_work, started);
        return result;
    }
    prior_work.absorb(&primary);

    let Some(remaining) = remaining_global_config(config, started, prior_work.visited_states) else {
        return budget_exhausted_result(config, prior_work, started);
    };
    let mut secondary = HkcBackend.analyze_binary(query, left, right, &remaining);
    merge_work(&mut secondary, prior_work, started);
    secondary
}

fn remaining_global_config(
    config: &Config,
    started: Instant,
    used_states: usize,
) -> Option<Config> {
    let elapsed = started.elapsed().as_millis();
    if elapsed >= u128::from(config.timeout_ms) || used_states >= config.max_product_states {
        return None;
    }
    let mut remaining = config.clone();
    remaining.max_product_states = config.max_product_states - used_states;
    remaining.timeout_ms = (u128::from(config.timeout_ms) - elapsed)
        .min(u128::from(u64::MAX)) as u64;
    remaining.timeout_ms = remaining.timeout_ms.max(1);
    Some(remaining)
}

fn budget_exhausted_result(config: &Config, work: Work, started: Instant) -> BackendResult {
    let status = if started.elapsed().as_millis() >= u128::from(config.timeout_ms) {
        BackendStatus::Timeout
    } else {
        BackendStatus::StateLimit
    };
    BackendResult {
        status,
        witness: None,
        relation: None,
        visited_states: work.visited_states,
        generated_transitions: work.generated_transitions,
        analysis_ms: started
            .elapsed()
            .as_millis()
            .saturating_sub(work.witness_extraction_ms),
        witness_extraction_ms: work.witness_extraction_ms,
    }
}

fn merge_work(result: &mut BackendResult, prior: Work, started: Instant) {
    result.visited_states = result.visited_states.saturating_add(prior.visited_states);
    result.generated_transitions = result
        .generated_transitions
        .saturating_add(prior.generated_transitions);
    result.witness_extraction_ms = result
        .witness_extraction_ms
        .saturating_add(prior.witness_extraction_ms);
    result.analysis_ms = started
        .elapsed()
        .as_millis()
        .saturating_sub(result.witness_extraction_ms);
}

fn raw_result(
    status: BackendStatus,
    witness: Option<String>,
    relation_name: Option<&str>,
    visited_states: usize,
    generated_transitions: usize,
    started: Instant,
    witness_extraction_ms: u128,
) -> BackendResult {
    BackendResult {
        status,
        witness,
        relation: relation_name.map(str::to_owned),
        visited_states,
        generated_transitions,
        analysis_ms: started
            .elapsed()
            .as_millis()
            .saturating_sub(witness_extraction_ms),
        witness_extraction_ms,
    }
}

fn is_definitive(status: BackendStatus) -> bool {
    matches!(status, BackendStatus::Found | BackendStatus::Exhausted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::analyze_binary_with_backend;
    use crate::report::Verdict;

    #[test]
    fn recognizes_the_two_window_separator() {
        let config = Config::default();
        let left = crate::parser::parse("[RW]*R[RW]{8}#[RW]{8}R[RW]*", &config).unwrap();
        let right = crate::parser::parse("[RW]*W[RW]{8}#[RW]{8}R[RW]*", &config).unwrap();
        let (left_split, right_split) = common_unique_separator(&left, &right).unwrap();
        assert_eq!(left_split.separator, '#');
        assert_eq!(right_split.separator, '#');
    }

    #[test]
    fn refuses_a_separator_that_can_also_appear_in_a_component() {
        let config = Config::default();
        let left = crate::parser::parse("[#a]*#b", &config).unwrap();
        let right = crate::parser::parse("a*#b", &config).unwrap();
        assert!(common_unique_separator(&left, &right).is_none());
    }

    #[test]
    fn composed_inclusion_witness_matches_automata() {
        let config = Config::default();
        let portfolio = analyze_binary_with_backend(
            Query::Includes,
            "(a|b)#(c|d)",
            "b#d",
            &config,
            &PortfolioBackend,
        )
        .unwrap();
        let automata = analyze_binary_with_backend(
            Query::Includes,
            "(a|b)#(c|d)",
            "b#d",
            &config,
            &AutomataBackend,
        )
        .unwrap();
        assert_eq!(portfolio.verdict, automata.verdict);
        assert_eq!(
            portfolio.witness.as_ref().map(|w| &w.value),
            automata.witness.as_ref().map(|w| &w.value)
        );
    }

    #[test]
    fn composed_equivalence_witness_matches_automata() {
        let config = Config::default();
        let portfolio = analyze_binary_with_backend(
            Query::Equivalent,
            "(a|b)#(c|d)",
            "b#c",
            &config,
            &PortfolioBackend,
        )
        .unwrap();
        let automata = analyze_binary_with_backend(
            Query::Equivalent,
            "(a|b)#(c|d)",
            "b#c",
            &config,
            &AutomataBackend,
        )
        .unwrap();
        assert_eq!(portfolio.verdict, automata.verdict);
        assert_eq!(
            portfolio.witness.as_ref().map(|w| &w.value),
            automata.witness.as_ref().map(|w| &w.value)
        );
    }

    #[test]
    fn separator_decomposition_matches_automata_on_small_cross_product() {
        let config = Config::default();
        let patterns = [
            "a#b",
            "(a|b)#b",
            "a#(b|c)",
            "(a|b)#(b|c)",
            "a*#b?",
            "#(a|b)",
        ];
        for left in patterns {
            for right in patterns {
                for query in [Query::Includes, Query::Equivalent] {
                    let portfolio = analyze_binary_with_backend(
                        query,
                        left,
                        right,
                        &config,
                        &PortfolioBackend,
                    )
                    .unwrap();
                    let automata = analyze_binary_with_backend(
                        query,
                        left,
                        right,
                        &config,
                        &AutomataBackend,
                    )
                    .unwrap();
                    assert_eq!(
                        portfolio.verdict, automata.verdict,
                        "verdict mismatch: {query:?} {left:?} {right:?}"
                    );
                    assert_eq!(
                        portfolio.witness.as_ref().map(|w| (&w.value, &w.relation)),
                        automata.witness.as_ref().map(|w| (&w.value, &w.relation)),
                        "witness mismatch: {query:?} {left:?} {right:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn separator_path_solves_larger_two_window_family() {
        let config = Config {
            max_product_states: 100_000,
            timeout_ms: 5_000,
            ..Config::default()
        };
        for n in [20, 32, 48, 96, 128, 256] {
            let left = format!("[RW]*R[RW]{{{n}}}#[RW]{{{n}}}R[RW]*");
            let right = format!("[RW]*W[RW]{{{n}}}#[RW]{{{n}}}R[RW]*");
            for query in [Query::Includes, Query::Equivalent] {
                let report = analyze_binary_with_backend(
                    query,
                    &left,
                    &right,
                    &config,
                    &PortfolioBackend,
                )
                .unwrap();
                assert_eq!(report.verdict, Verdict::No, "n={n}, query={query:?}");
            }
        }
    }

    #[test]
    fn portfolio_never_refreshes_the_state_budget_between_engines() {
        let config = Config {
            max_product_states: 1,
            timeout_ms: 1_000,
            ..Config::default()
        };
        let report = analyze_binary_with_backend(
            Query::Includes,
            "a|b",
            "a",
            &config,
            &PortfolioBackend,
        )
        .unwrap();
        assert!(report.statistics.visited_product_states <= 1);
    }

    #[test]
    fn shortest_nfa_word_does_not_determinize_window_history() {
        let config = Config {
            max_product_states: 1_000,
            timeout_ms: 1_000,
            ..Config::default()
        };
        let expr = crate::parser::parse("[RW]*R[RW]{128}", &config).unwrap();
        let nfa = Nfa::from_expr(&expr);
        let result = shortest_nfa_word(&nfa, &config);
        assert_eq!(result.status, BackendStatus::Found);
        assert_eq!(result.witness.as_ref().map(|w| w.chars().count()), Some(129));
        assert!(result.visited_states <= nfa.states.len());
    }
}
