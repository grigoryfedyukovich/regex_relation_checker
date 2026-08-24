//! Common-subexpression abstraction and a lightweight CEGAR driver.
//!
//! Soundness (homomorphism argument):
//! - Abstract YES for Equivalent / Includes / Overlap / Empty ⇒ concrete YES.
//! - Abstract NO is inconclusive and triggers refinement or fall-back.
//!
//! The homomorphism argument requires each fresh marker `σ` to satisfy
//! `σ ∉ Σ` (the configured alphabet) -- otherwise `σ` is just an ordinary
//! character, and a real occurrence of it anywhere in either pattern
//! collides with the substitution, breaking the `h(σ) = L(S)` argument the
//! rest of this module relies on. `Alphabet::Unicode`'s declared scalar
//! range is every valid Unicode scalar value there is, so it leaves no `σ`
//! free; see `alphabet_has_room_for_markers`, checked once per call before
//! any abstraction is attempted.
//!
//! Strategy:
//! 1. Collect structurally identical subexpressions that appear in both ASTs.
//! 2. Replace the largest ones by fresh letters (outside the active alphabet).
//! 3. Run an inner backend on the abstracted pair.
//! 4. On YES → accept. On NO/UNKNOWN → expand sites indicated by a
//!    distinguishing witness (if any) or the largest remaining sites, then
//!    retry. After a fixed budget, fall back to the concrete inner analysis.

use crate::analysis::{BackendResult, BackendStatus, Query, RelationBackend};
use crate::ast::{Expr, ExprKind, Span};
use crate::charset::{CharSet, Interval};
use crate::config::{Alphabet, Config};
use crate::nfa::Nfa;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

/// Private-use area start. Markers are ordinary `char` literals drawn
/// upward from here, so they are only actually fresh -- disjoint from every
/// pattern's real characters -- when the configured alphabet's scalar range
/// stops below this point. See `alphabet_has_room_for_markers`.
const FRESH_BASE: u32 = 0xE000;

/// Maximum CEGAR refinement rounds before falling back to the concrete backend.
const MAX_REFINEMENT_ROUNDS: usize = 4;

/// Minimum structural size for a subexpression to be worth abstracting.
const MIN_ABSTRACT_SIZE: usize = 3;

/// Whether `alphabet` leaves any scalar value free for a marker.
///
/// Markers are encoded as ordinary `char` literals starting at
/// `FRESH_BASE` and counting upward (`build_initial_map`), so this checks
/// whether `alphabet`'s own declared scalar range (`Alphabet::scalar_intervals`)
/// reaches into `FRESH_BASE..=0x10FFFF` at all -- if it does, some marker
/// value collides with a real character *of that same alphabet*, and no
/// choice of marker in that range is actually outside `Σ`.
///
/// `Alphabet::Ascii` tops out at `U+007F`, well below `FRESH_BASE`, so the
/// whole marker range is free. `Alphabet::Unicode`'s range is `U+0000..U+D7FF`
/// ∪ `U+E000..U+10FFFF` -- precisely the set of every value a Rust `char` can
/// hold, since surrogates are excluded from both `char` and this alphabet
/// for the same reason -- so it overlaps the marker range entirely and
/// leaves nothing free. This is computed against the actual declared range
/// rather than hard-coded per variant, so a future alphabet is checked by
/// the same rule instead of silently defaulting to "safe".
fn alphabet_has_room_for_markers(alphabet: Alphabet) -> bool {
    let universe = CharSet::from_u32_intervals(alphabet.scalar_intervals().to_vec());
    let marker_region = CharSet::from_u32_intervals(vec![Interval::new(FRESH_BASE, 0x10ffff)]);
    universe.intersect(&marker_region).is_empty()
}

/// Score used to prefer large / expensive common subexpressions.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SizeScore(usize);

fn expr_size(expr: &Expr) -> SizeScore {
    match &expr.kind {
        ExprKind::Empty
        | ExprKind::Literal(_)
        | ExprKind::CharSet(_)
        | ExprKind::AnchorStart
        | ExprKind::AnchorEnd => SizeScore(1),
        ExprKind::Concat(parts) | ExprKind::Alt(parts) => {
            SizeScore(1 + parts.iter().map(|p| expr_size(p).0).sum::<usize>())
        }
        ExprKind::Repeat { expr, min, max } => {
            let inner = expr_size(expr).0;
            let bound = max.unwrap_or(*min).saturating_add(1).max(2);
            // Weight counted / starred constructs more heavily.
            SizeScore(1 + inner * bound.min(8))
        }
    }
}

/// Light structural normalisation so that e.g. `(a|b)` and `(b|a)` share a key.
fn normalize(expr: &Expr) -> Expr {
    let span = expr.span;
    match &expr.kind {
        ExprKind::Concat(parts) => {
            let mut flat = Vec::new();
            for p in parts {
                let n = normalize(p);
                if let ExprKind::Concat(inner) = n.kind {
                    flat.extend(inner);
                } else if !matches!(n.kind, ExprKind::Empty) {
                    flat.push(n);
                }
            }
            match flat.len() {
                0 => Expr::new(ExprKind::Empty, span),
                1 => flat.pop().unwrap(),
                _ => Expr::new(ExprKind::Concat(flat), span),
            }
        }
        ExprKind::Alt(parts) => {
            let mut flat: Vec<Expr> = parts.iter().map(normalize).collect();
            // Sort by Debug representation for deterministic equality.
            flat.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
            flat.dedup_by(|a, b| a.kind == b.kind);
            match flat.len() {
                0 => Expr::new(ExprKind::Empty, span),
                1 => flat.pop().unwrap(),
                _ => Expr::new(ExprKind::Alt(flat), span),
            }
        }
        ExprKind::Repeat { expr, min, max } => Expr::new(
            ExprKind::Repeat {
                expr: Box::new(normalize(expr)),
                min: *min,
                max: *max,
            },
            span,
        ),
        _ => expr.clone(),
    }
}

/// Structural key for common-subexpression discovery and matching (ignores
/// spans -- see `zero_spans`).
fn structural_key(expr: &Expr) -> String {
    format!("{:?}", zero_spans(expr).kind)
}

/// Clone of `expr` with every span -- this node's own and every
/// descendant's -- reset to a canonical value.
///
/// `structural_key` only ever formats `.kind`, never the whole `Expr`, so
/// this node's *own* span was already excluded -- but `ExprKind::Concat`,
/// `Alt`, and `Repeat` all embed full child `Expr` values (kind *and*
/// span), and a derived `Debug` impl formats those recursively. Without
/// this, two structurally-identical subexpressions occurring at different
/// byte offsets in the source text -- the common case for any subexpression
/// repeated more than once in a pattern -- would carry different span
/// values down through their descendants and so produce different keys,
/// silently failing to match despite being the exact same shape.
fn zero_spans(expr: &Expr) -> Expr {
    let kind = match &expr.kind {
        ExprKind::Concat(parts) => ExprKind::Concat(parts.iter().map(zero_spans).collect()),
        ExprKind::Alt(parts) => ExprKind::Alt(parts.iter().map(zero_spans).collect()),
        ExprKind::Repeat { expr, min, max } => ExprKind::Repeat {
            expr: Box::new(zero_spans(expr)),
            min: *min,
            max: *max,
        },
        other => other.clone(),
    };
    Expr::new(kind, Span::new(0, 0))
}

/// Collect every subexpression (post-normalisation) together with its size.
fn collect_subexprs(expr: &Expr, out: &mut HashMap<String, (Expr, SizeScore)>) {
    let key = structural_key(expr);
    let score = expr_size(expr);
    out.entry(key)
        .and_modify(|e| {
            if score > e.1 {
                *e = (expr.clone(), score);
            }
        })
        .or_insert_with(|| (expr.clone(), score));

    match &expr.kind {
        ExprKind::Concat(parts) | ExprKind::Alt(parts) => {
            for p in parts {
                collect_subexprs(p, out);
            }
        }
        ExprKind::Repeat { expr, .. } => collect_subexprs(expr, out),
        _ => {}
    }
}

/// Sites that were abstracted: fresh char → original subexpression.
#[derive(Clone, Debug, Default)]
struct AbstractionMap {
    /// fresh char → (original expr, size)
    entries: HashMap<char, (Expr, SizeScore)>,
    /// ordered list of fresh chars (largest first)
    order: Vec<char>,
}

impl AbstractionMap {
    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn expand_one(&mut self, ch: char) -> bool {
        if self.entries.remove(&ch).is_some() {
            self.order.retain(|&c| c != ch);
            true
        } else {
            false
        }
    }

    fn expand_largest(&mut self) -> bool {
        if let Some(ch) = self.order.first().copied() {
            self.expand_one(ch)
        } else {
            false
        }
    }

    /// Expand every fresh symbol that appears in the given witness string.
    fn expand_from_witness(&mut self, witness: &str) -> usize {
        let used: HashSet<char> = witness
            .chars()
            .filter(|c| self.entries.contains_key(c))
            .collect();
        let mut count = 0;
        for ch in used {
            if self.expand_one(ch) {
                count += 1;
            }
        }
        count
    }
}

/// Substitute every fresh marker in `witness` with a genuine shortest string
/// drawn from the language of the subexpression it stands for, producing a
/// witness over the *original* alphabet that can be replayed against the
/// concrete (unabstracted) automata.
///
/// This is the concrete counterpart of the homomorphism `h` used in the
/// soundness argument: `h(sigma) = L(S)`. Returns `None` if some marker in
/// the witness stands for a subexpression whose language is empty — in that
/// case no concrete substitution exists, the abstract "witness" does not
/// correspond to any real string, and the caller must not treat the abstract
/// verdict as sound.
fn expand_witness(witness: &str, map: &AbstractionMap, config: &Config) -> Option<String> {
    let mut cache: HashMap<char, String> = HashMap::new();
    let mut out = String::with_capacity(witness.len());
    for ch in witness.chars() {
        match map.entries.get(&ch) {
            None => out.push(ch),
            Some((sub_expr, _)) => {
                if let Some(piece) = cache.get(&ch) {
                    out.push_str(piece);
                    continue;
                }
                let sub_nfa = Nfa::from_expr(sub_expr);
                let sub_result = crate::analysis::search_single(&sub_nfa, config);
                match (sub_result.status, sub_result.witness) {
                    (BackendStatus::Found, Some(piece)) => {
                        out.push_str(&piece);
                        cache.insert(ch, piece);
                    }
                    // Exhausted-with-no-witness (or a stopped search) means the
                    // abstracted subexpression's language is empty or could not
                    // be shown non-empty within budget: this marker cannot be
                    // soundly substituted back.
                    _ => return None,
                }
            }
        }
    }
    Some(out)
}

/// Replace every occurrence of the selected subexpressions by the corresponding fresh literal.
fn apply_abstraction(expr: &Expr, map: &AbstractionMap) -> Expr {
    // Prefer larger matches first: try each abstractable subexpr in size order.
    for &ch in &map.order {
        if let Some((ref target, _)) = map.entries.get(&ch) {
            if structural_key(expr) == structural_key(target) {
                return Expr::new(ExprKind::Literal(ch), expr.span);
            }
        }
    }
    let span = expr.span;
    match &expr.kind {
        ExprKind::Concat(parts) => {
            let new_parts: Vec<_> = parts.iter().map(|p| apply_abstraction(p, map)).collect();
            Expr::new(ExprKind::Concat(new_parts), span)
        }
        ExprKind::Alt(parts) => {
            let new_parts: Vec<_> = parts.iter().map(|p| apply_abstraction(p, map)).collect();
            Expr::new(ExprKind::Alt(new_parts), span)
        }
        ExprKind::Repeat { expr, min, max } => Expr::new(
            ExprKind::Repeat {
                expr: Box::new(apply_abstraction(expr, map)),
                min: *min,
                max: *max,
            },
            span,
        ),
        _ => expr.clone(),
    }
}

/// Build the initial abstraction map: common subexprs, largest-first, limited count.
fn build_initial_map(left: &Expr, right: &Expr, max_abstractions: usize) -> AbstractionMap {
    let left_n = normalize(left);
    let right_n = normalize(right);

    let mut left_subs = HashMap::new();
    let mut right_subs = HashMap::new();
    collect_subexprs(&left_n, &mut left_subs);
    collect_subexprs(&right_n, &mut right_subs);

    let mut common: Vec<(String, Expr, SizeScore)> = Vec::new();
    for (key, (expr, score)) in left_subs {
        if score.0 < MIN_ABSTRACT_SIZE {
            continue;
        }
        if right_subs.contains_key(&key) {
            common.push((key, expr, score));
        }
    }
    // Largest first; ties broken by structural key so the choice of which
    // equal-size common subexpressions get abstracted (and in what order) is
    // deterministic across runs, independent of HashMap iteration order.
    common.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));

    let mut map = AbstractionMap::default();
    let mut next_fresh = FRESH_BASE;
    for (_, expr, score) in common.into_iter().take(max_abstractions) {
        let ch = char::from_u32(next_fresh).unwrap_or('\u{E000}');
        next_fresh = next_fresh.saturating_add(1);
        map.entries.insert(ch, (expr, score));
        map.order.push(ch);
    }
    map
}

/// Rebuild abstracted expressions from the current map.
fn abstract_pair(left: &Expr, right: &Expr, map: &AbstractionMap) -> (Expr, Expr) {
    (apply_abstraction(left, map), apply_abstraction(right, map))
}

/// Map abstract status into the concrete Report-level decision we can trust.
fn abstract_verdict(query: Query, result: &BackendResult) -> Option<bool> {
    // Returns Some(true) = YES, Some(false) = NO, None = inconclusive
    match result.status {
        BackendStatus::StateLimit | BackendStatus::Timeout => None,
        BackendStatus::Found => match query {
            Query::Overlap => Some(true),
            Query::Empty => Some(false), // word found → not empty
            Query::Includes | Query::Equivalent => Some(false), // witness → NO
            Query::Match => Some(true),
        },
        BackendStatus::Exhausted => match query {
            Query::Overlap => Some(false),
            Query::Empty => Some(true),
            Query::Includes | Query::Equivalent => Some(true),
            Query::Match => Some(false),
        },
    }
}

/// Clone `config` with `timeout_ms` reduced to whatever remains of the
/// *original* caller-configured budget, measured from `started`. Every
/// internal call the CEGAR driver makes (abstracted rounds, witness
/// expansion, concrete fallback) must share this one deadline — otherwise
/// each round re-arms a fresh full timeout and a pathological input could
/// consume up to `(MAX_REFINEMENT_ROUNDS + 1) x timeout_ms` wall-clock time
/// before ever reporting `UNKNOWN`, silently blowing past the budget the
/// caller configured.
fn budget_config(config: &Config, started: Instant) -> Config {
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let remaining = config.timeout_ms.saturating_sub(elapsed_ms).max(1);
    let mut cfg = config.clone();
    cfg.timeout_ms = remaining;
    cfg
}

/// CEGAR driver that owns the abstraction map and delegates to an inner backend.
pub struct AbstractionBackend<B: RelationBackend> {
    pub inner: B,
    /// Max number of common subexpressions to abstract initially.
    pub max_abstractions: usize,
}

impl AbstractionBackend<crate::analysis::AutomataBackend> {
    /// Convenience constructor: CEGAR over the default automata backend.
    pub fn new() -> Self {
        Self::with_inner(crate::analysis::AutomataBackend)
    }
}

impl Default for AbstractionBackend<crate::analysis::AutomataBackend> {
    fn default() -> Self {
        Self::new()
    }
}

impl<B: RelationBackend> AbstractionBackend<B> {
    /// Build a CEGAR driver that delegates abstract rounds and the concrete
    /// fall-back to `inner`. Uses the default abstraction budget (8).
    pub fn with_inner(inner: B) -> Self {
        Self {
            inner,
            max_abstractions: 8,
        }
    }

    /// Like [`with_inner`] but with an explicit initial abstraction budget.
    pub fn with_inner_and_budget(inner: B, max_abstractions: usize) -> Self {
        Self {
            inner,
            max_abstractions,
        }
    }
}

impl<B: RelationBackend> RelationBackend for AbstractionBackend<B> {
    fn name(&self) -> &'static str {
        // Stable short label so existing tests / JSON consumers that key on
        // "abstraction" keep working. The concrete inner is chosen at
        // construction time (`with_inner` / CLI `--abstraction-inner`).
        "abstraction"
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
        // Without AST we cannot discover common subexpressions; fall through.
        self.inner.analyze_binary(query, left, right, config)
    }

    fn analyze_empty(&self, nfa: &Nfa, config: &Config) -> BackendResult {
        self.inner.analyze_empty(nfa, config)
    }

    fn analyze_binary_expr(
        &self,
        query: Query,
        left_expr: &Expr,
        right_expr: &Expr,
        _left_nfa: &Nfa,
        _right_nfa: &Nfa,
        config: &Config,
    ) -> BackendResult {
        let started = Instant::now();

        if !alphabet_has_room_for_markers(config.alphabet) {
            // No scalar value is free for a marker under this alphabet (see
            // `alphabet_has_room_for_markers`), so any abstraction here
            // could substitute a marker that collides with a real
            // character occurring in either pattern -- the homomorphism
            // argument this module relies on no longer holds. Skip
            // straight to concrete analysis rather than risk trusting an
            // unsound abstract YES; this is the same fallback path used
            // below when there's nothing to abstract or refinement is
            // exhausted.
            let left_nfa = Nfa::from_expr(left_expr);
            let right_nfa = Nfa::from_expr(right_expr);
            return self.inner.analyze_binary_expr(
                query,
                left_expr,
                right_expr,
                &left_nfa,
                &right_nfa,
                &budget_config(config, started),
            );
        }

        let mut map = build_initial_map(left_expr, right_expr, self.max_abstractions);

        if map.is_empty() {
            // Nothing useful to abstract → concrete analysis.
            let left_nfa = Nfa::from_expr(left_expr);
            let right_nfa = Nfa::from_expr(right_expr);
            return self.inner.analyze_binary_expr(
                query,
                left_expr,
                right_expr,
                &left_nfa,
                &right_nfa,
                &budget_config(config, started),
            );
        }

        // `build_initial_map` discovers common subexpressions from the
        // *normalized* trees (alt branches sorted, concats flattened) --
        // its map targets are normalized nodes. `apply_abstraction` (via
        // `abstract_pair`) must walk that same normalized shape, or a node
        // written in a different-but-equivalent order in the original
        // source (`(b|a)` against a map entry discovered as `(a|b)`) fails
        // the structural-key match and silently isn't replaced -- normalize
        // is a pure language-preserving canonicalization (alternation is
        // commutative and idempotent; concatenation is associative and `ε`
        // is its identity), so working from `left_n`/`right_n` from here on
        // changes nothing about what language gets analyzed.
        let left_n = normalize(left_expr);
        let right_n = normalize(right_expr);

        let mut rounds = 0;
        loop {
            let (abs_left, abs_right) = abstract_pair(&left_n, &right_n, &map);
            let left_nfa = Nfa::from_expr(&abs_left);
            let right_nfa = Nfa::from_expr(&abs_right);
            let round_config = budget_config(config, started);

            let result = self.inner.analyze_binary_expr(
                query,
                &abs_left,
                &abs_right,
                &left_nfa,
                &right_nfa,
                &round_config,
            );

            let mut verdict = abstract_verdict(query, &result);
            // A "sound YES" that carries a witness (Overlap/Match) is only
            // actually sound once every fresh marker in that witness is
            // expanded back into a genuine substring of the subexpression it
            // stands for. If some marker turns out to stand for a
            // subexpression with an empty language, no such substitution
            // exists and the witness does not correspond to any real string;
            // treat that as inconclusive rather than trusting it.
            let mut expanded_witness: Option<String> = None;
            if verdict == Some(true) {
                if let Some(w) = &result.witness {
                    match expand_witness(w, &map, &budget_config(config, started)) {
                        Some(concrete) => expanded_witness = Some(concrete),
                        None => verdict = None,
                    }
                }
            }

            match verdict {
                // Sound YES — accept immediately, with any fresh markers
                // already expanded back into real substrings so the witness
                // replays against the original (unabstracted) automata.
                Some(true) => {
                    return BackendResult {
                        status: result.status,
                        witness: expanded_witness.or(result.witness),
                        relation: result.relation,
                        visited_states: result.visited_states,
                        generated_transitions: result.generated_transitions,
                        analysis_ms: started.elapsed().as_millis(),
                        witness_extraction_ms: result.witness_extraction_ms,
                    };
                }
                // Abstract NO or UNKNOWN → try to refine.
                Some(false) | None => {
                    rounds += 1;
                    if rounds > MAX_REFINEMENT_ROUNDS || map.is_empty() {
                        // Fall back to concrete analysis.
                        let left_nfa = Nfa::from_expr(left_expr);
                        let right_nfa = Nfa::from_expr(right_expr);
                        return self.inner.analyze_binary_expr(
                            query,
                            left_expr,
                            right_expr,
                            &left_nfa,
                            &right_nfa,
                            &budget_config(config, started),
                        );
                    }

                    // Prefer counterexample-guided expansion when a witness is present.
                    let expanded = if let Some(ref w) = result.witness {
                        map.expand_from_witness(w)
                    } else {
                        0
                    };
                    if expanded == 0 {
                        // No useful witness info → expand largest remaining site.
                        if !map.expand_largest() {
                            // Nothing left to expand.
                            let left_nfa = Nfa::from_expr(left_expr);
                            let right_nfa = Nfa::from_expr(right_expr);
                            return self.inner.analyze_binary_expr(
                                query,
                                left_expr,
                                right_expr,
                                &left_nfa,
                                &right_nfa,
                                &budget_config(config, started),
                            );
                        }
                    }
                    // Continue CEGAR loop with the refined map.
                }
            }
        }
    }

    fn analyze_empty_expr(&self, expr: &Expr, nfa: &Nfa, config: &Config) -> BackendResult {
        // Emptiness has only one expression; common-subexpression abstraction
        // across two patterns does not apply. Delegate.
        self.inner.analyze_empty_expr(expr, nfa, config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{analyze_binary_with_backend, Query};
    use crate::config::Config;
    use crate::parser::parse;
    use crate::report::Verdict;

    #[test]
    fn ascii_alphabet_leaves_room_for_markers_unicode_does_not() {
        assert!(alphabet_has_room_for_markers(Alphabet::Ascii));
        assert!(!alphabet_has_room_for_markers(Alphabet::Unicode));
    }

    /// Direct regression test for the marker-collision mechanism in the bug
    /// report, built by hand rather than via `build_initial_map`'s
    /// auto-discovery. (`build_initial_map`/`apply_abstraction` only
    /// replace *every* occurrence of a shared subexpression when they sit
    /// at byte-identical spans across both patterns -- an orthogonal,
    /// pre-existing quirk of `structural_key` unrelated to marker
    /// freshness, and not always true for hand-written examples like the
    /// one in the report. Building the map directly sidesteps that and
    /// tests the actual mechanism at risk.)
    ///
    /// `shared` (`(x|y|z)`) is abstracted to `FRESH_BASE` (U+E000) in both
    /// `left = shared · U+E000` and `right = shared · shared`. Because
    /// U+E000 is *also* the marker value, `left`'s trailing literal already
    /// equals the marker without any substitution -- so both abstracted
    /// trees collapse to the identical two-marker string "σσ", and running
    /// an inner backend on them directly reports Equivalent = YES, even
    /// though the concrete languages (`x`/`y`/`z` followed by U+E000, vs.
    /// `x`/`y`/`z` followed by `x`/`y`/`z`) are disjoint. This is exactly
    /// the hazard `alphabet_has_room_for_markers` exists to shut out; the
    /// second half of this test confirms the real `AbstractionBackend`
    /// does shut it out under `Alphabet::Unicode`.
    #[test]
    fn hand_built_marker_collision_would_be_unsound_without_the_alphabet_gate() {
        let shared = Expr::new(
            ExprKind::Alt(vec![
                Expr::new(ExprKind::Literal('x'), Span::new(0, 0)),
                Expr::new(ExprKind::Literal('y'), Span::new(0, 0)),
                Expr::new(ExprKind::Literal('z'), Span::new(0, 0)),
            ]),
            Span::new(0, 0),
        );
        let marker = char::from_u32(FRESH_BASE).unwrap();
        let left_expr = Expr::new(
            ExprKind::Concat(vec![
                shared.clone(),
                Expr::new(ExprKind::Literal(marker), Span::new(0, 0)),
            ]),
            Span::new(0, 0),
        );
        let right_expr = Expr::new(
            ExprKind::Concat(vec![shared.clone(), shared.clone()]),
            Span::new(0, 0),
        );

        // Hand-build the map the same way `build_initial_map` would if it
        // had found this candidate.
        let mut map = AbstractionMap::default();
        map.entries
            .insert(marker, (shared.clone(), expr_size(&shared)));
        map.order.push(marker);
        let (abs_left, abs_right) = abstract_pair(&left_expr, &right_expr, &map);

        // Both abstracted trees collapse to the same two-marker string.
        assert_eq!(structural_key(&abs_left), structural_key(&abs_right));

        let cfg = Config {
            alphabet: Alphabet::Unicode,
            ..Config::default()
        };

        // Confirm the abstract analysis really would call this a sound YES...
        let abs_left_nfa = Nfa::from_expr(&abs_left);
        let abs_right_nfa = Nfa::from_expr(&abs_right);
        let abstract_result = crate::analysis::AutomataBackend.analyze_binary(
            Query::Equivalent,
            &abs_left_nfa,
            &abs_right_nfa,
            &cfg,
        );
        assert_eq!(
            abstract_verdict(Query::Equivalent, &abstract_result),
            Some(true),
            "sanity: the hand-built abstraction really does look like a sound YES"
        );

        // ...while the concrete languages are actually disjoint.
        let left_nfa = Nfa::from_expr(&left_expr);
        let right_nfa = Nfa::from_expr(&right_expr);
        let concrete_result = crate::analysis::AutomataBackend.analyze_binary(
            Query::Equivalent,
            &left_nfa,
            &right_nfa,
            &cfg,
        );
        assert_eq!(
            abstract_verdict(Query::Equivalent, &concrete_result),
            Some(false),
            "sanity: the concrete languages actually differ"
        );

        // The real driver must not walk into this trap under Alphabet::Unicode.
        let backend = AbstractionBackend::new();
        let real_result = backend.analyze_binary_expr(
            Query::Equivalent,
            &left_expr,
            &right_expr,
            &left_nfa,
            &right_nfa,
            &cfg,
        );
        assert_eq!(
            abstract_verdict(Query::Equivalent, &real_result),
            Some(false),
            "AbstractionBackend must not trust the colliding abstraction under Alphabet::Unicode"
        );
    }

    #[test]
    fn structural_key_ignores_descendant_spans() {
        // Same shape, different source positions (as `(x|y|z)` occurring
        // twice in one pattern would produce): keys must match regardless.
        let a = Expr::new(
            ExprKind::Alt(vec![
                Expr::new(ExprKind::Literal('x'), Span::new(0, 1)),
                Expr::new(ExprKind::Literal('y'), Span::new(2, 3)),
            ]),
            Span::new(0, 4),
        );
        let b = Expr::new(
            ExprKind::Alt(vec![
                Expr::new(ExprKind::Literal('x'), Span::new(100, 101)),
                Expr::new(ExprKind::Literal('y'), Span::new(200, 201)),
            ]),
            Span::new(50, 250),
        );
        assert_eq!(structural_key(&a), structural_key(&b));
    }

    /// Regression test: `build_initial_map` discovers common subexpressions
    /// from *normalized* trees (alt branches sorted), but used to hand
    /// `apply_abstraction` the original, un-normalized trees to match
    /// against. `(b|a)`, written in source order, never matched a map
    /// entry discovered (and stored) as the normalized `(a|b)` -- so a
    /// subexpression written in a different-but-equivalent branch order on
    /// one side either silently skipped abstraction (safe, just slower) or,
    /// combined with the `structural_key` span leak, could abstract only
    /// one side. `analyze_binary_expr` now matches against `left_n`/`right_n`
    /// (both normalized), so this must fully collapse instead of falling
    /// through to the automata-sized product this pair would otherwise need.
    #[test]
    fn abstracts_a_subexpression_written_in_different_branch_order() {
        let cfg = Config::default();
        let backend = AbstractionBackend::new();
        let left = "((a|b|c|d|e){25}f){10}x";
        let right = "((e|d|c|b|a){25}f){10}x"; // same alternation, reversed order
        let report =
            analyze_binary_with_backend(Query::Equivalent, left, right, &cfg, &backend).unwrap();
        assert_eq!(report.verdict, Verdict::Yes);
        // A full concrete product for this pair visits >1000 states (see
        // the sibling automata-backend comparison in this test file's
        // history); successful abstraction collapses both repeated blocks
        // to a single shared marker, leaving a tiny product.
        assert!(
            report.statistics.visited_product_states < 10,
            "expected the shared, differently-ordered alternation to collapse via \
             abstraction, got {} product states",
            report.statistics.visited_product_states
        );
    }

    #[test]
    fn discovers_common_star() {
        let cfg = Config::default();
        let left = parse("(ab)*c", &cfg).unwrap();
        let right = parse("(ab)*d", &cfg).unwrap();
        let map = build_initial_map(&left, &right, 4);
        assert!(!map.is_empty(), "expected to abstract the shared (ab)*");
    }

    #[test]
    fn sound_yes_equivalence() {
        let cfg = Config::default();
        let backend = AbstractionBackend::new();
        // Identical patterns → should quickly return YES via abstraction or concrete.
        let report =
            analyze_binary_with_backend(Query::Equivalent, "a(bc)*d", "a(bc)*d", &cfg, &backend)
                .unwrap();
        assert_eq!(report.verdict, Verdict::Yes);
    }

    /// Regression test: an Overlap sound-YES used to return the raw abstract
    /// witness (containing an unexpanded private-use-area marker standing in
    /// for the shared `(bc)*`), which then failed replay against the
    /// original, unabstracted automata at the top level and surfaced as an
    /// internal error instead of a verdict. Every character of a witness
    /// returned through the public API must belong to the original pattern's
    /// alphabet, never a fresh abstraction marker.
    #[test]
    fn overlap_witness_never_contains_fresh_abstraction_markers() {
        let cfg = Config::default();
        let backend = AbstractionBackend::new();
        let report =
            analyze_binary_with_backend(Query::Overlap, "a(bc)*d", "a(bc)*d", &cfg, &backend)
                .unwrap();
        assert_eq!(report.verdict, Verdict::Yes);
        let witness = report
            .witness
            .as_ref()
            .expect("overlap YES must carry a witness")
            .value
            .clone();
        assert!(
            witness.chars().all(|c| (c as u32) < FRESH_BASE),
            "witness {witness:?} leaked a fresh abstraction marker"
        );
        // "ad" (zero repetitions of the shared block) is the shortest witness.
        assert_eq!(witness, "ad");
    }

    /// Regression test: a structurally shared subexpression whose language
    /// is empty (a dead branch) must never be trusted as a valid
    /// abstraction witness. Before the fix, the abstracted search could
    /// treat the fresh marker for such a subexpression as an ordinary,
    /// always-matchable symbol and report a spurious overlap.
    #[test]
    fn empty_language_common_subexpression_does_not_cause_a_false_overlap() {
        let cfg = Config::default();
        let backend = AbstractionBackend::new();
        // `[^\d\D]` is the empty character class (complement of the whole
        // alphabet); `a[^\d\D]b` is a dead branch shared verbatim by both
        // patterns, whose only live behavior is `x` vs `y` — disjoint.
        let report = analyze_binary_with_backend(
            Query::Overlap,
            r"(a[^\d\D]b)|x",
            r"(a[^\d\D]b)|y",
            &cfg,
            &backend,
        )
        .unwrap();
        assert_eq!(report.verdict, Verdict::No);
    }

    /// Regression test: each CEGAR round used to re-arm a fresh, full
    /// `timeout_ms` budget, so a hard instance could take up to
    /// `(MAX_REFINEMENT_ROUNDS + 1) x timeout_ms` wall-clock time before
    /// returning UNKNOWN. The whole call must respect one shared deadline
    /// measured from the start of `analyze_binary_expr`.
    #[test]
    fn refinement_rounds_share_a_single_timeout_budget() {
        let cfg = Config {
            timeout_ms: 50,
            max_product_states: 200_000,
            ..Config::default()
        };
        let backend = AbstractionBackend::new();
        let started = Instant::now();
        let _ = analyze_binary_with_backend(
            Query::Overlap,
            "((a|b){25}c){10}x",
            "((a|b){25}c){10}y",
            &cfg,
            &backend,
        )
        .unwrap();
        let elapsed = started.elapsed().as_millis();
        // Generous slack over the 50ms budget for process/allocation
        // overhead, but nowhere near the ~250ms that 5 unshared rounds
        // would allow.
        assert!(
            elapsed < 400,
            "analysis took {elapsed}ms, budget was {}ms across all rounds",
            cfg.timeout_ms
        );
    }

    #[test]
    fn falls_back_on_non_equivalent() {
        let cfg = Config::default();
        let backend = AbstractionBackend::new();
        let report =
            analyze_binary_with_backend(Query::Equivalent, "a(bc)*d", "a(bc)*e", &cfg, &backend)
                .unwrap();
        assert_eq!(report.verdict, Verdict::No);
    }

    #[test]
    fn with_inner_derivatives_agrees_on_shared_block() {
        // Library API: CEGAR over Brzozowski residuals instead of automata.
        let cfg = Config::default();
        let backend = AbstractionBackend::with_inner(crate::derivative::DerivativeBackend);
        let report = analyze_binary_with_backend(
            Query::Overlap,
            "((a|b){8}c){3}x",
            "((a|b){8}c){3}x",
            &cfg,
            &backend,
        )
        .unwrap();
        assert_eq!(report.verdict, Verdict::Yes);
        assert_eq!(report.backend.name, "abstraction");
    }
}
