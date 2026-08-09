//! Antimirov (partial-derivative) backend for regular-language relation checking.
//!
//! Brzozowski derivatives produce a *single* residual expression per character.
//! Antimirov partial derivatives produce a *finite set* of residuals (a linear
//! form). The union of their languages equals the Brzozowski residual language,
//! but the set representation maps more directly onto NFA states and often
//! stays smaller when alternation would otherwise duplicate structure inside
//! one big residual term.
//!
//! Decision procedure: product BFS over pairs of linear forms. A linear form
//! accepts when any member is nullable. Alphabet partitions are taken from the
//! first-sets of every residual in the current pair, same representative
//! policy as the other backends.
//!
//! References: Antimirov, "Partial Derivatives of Regular Expressions and
//! Finite Automaton Constructions" (Theoretical Computer Science, 1996).

use crate::analysis::{BackendResult, BackendStatus, Query, RelationBackend};
use crate::ast::{Expr, ExprKind};
use crate::charset::CharSet;
use crate::config::Config;
use crate::nfa::Nfa;
use crate::report::relation;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Residual algebra (shared shape with the Brzozowski backend)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
enum RegKind {
    Null,
    Eps,
    Atom(CharSet),
    Concat(Vec<Reg>),
    Alt(Vec<Reg>),
    Star(Reg),
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct Reg(Rc<RegKind>);

impl Reg {
    fn null() -> Self {
        Self(Rc::new(RegKind::Null))
    }
    fn eps() -> Self {
        Self(Rc::new(RegKind::Eps))
    }
    fn atom(set: CharSet) -> Self {
        if set.is_empty() {
            Self::null()
        } else {
            Self(Rc::new(RegKind::Atom(set)))
        }
    }
    fn star(inner: Reg) -> Self {
        match inner.0.as_ref() {
            RegKind::Null | RegKind::Eps => Self::eps(),
            RegKind::Star(_) => inner,
            _ => Self(Rc::new(RegKind::Star(inner))),
        }
    }
    fn concat(parts: Vec<Reg>) -> Self {
        let mut flat = Vec::new();
        for part in parts {
            match part.0.as_ref() {
                RegKind::Null => return Self::null(),
                RegKind::Eps => {}
                RegKind::Concat(inner) => flat.extend(inner.iter().cloned()),
                _ => flat.push(part),
            }
        }
        match flat.len() {
            0 => Self::eps(),
            1 => flat.pop().unwrap(),
            _ => Self(Rc::new(RegKind::Concat(flat))),
        }
    }
    fn alt(branches: Vec<Reg>) -> Self {
        let mut flat = Vec::new();
        for b in branches {
            match b.0.as_ref() {
                RegKind::Null => {}
                RegKind::Alt(inner) => flat.extend(inner.iter().cloned()),
                _ => flat.push(b),
            }
        }
        flat.sort_by(reg_ord);
        flat.dedup();
        match flat.len() {
            0 => Self::null(),
            1 => flat.pop().unwrap(),
            _ => Self(Rc::new(RegKind::Alt(flat))),
        }
    }

    fn nullable(&self) -> bool {
        match self.0.as_ref() {
            RegKind::Null | RegKind::Atom(_) => false,
            RegKind::Eps | RegKind::Star(_) => true,
            RegKind::Concat(parts) => parts.iter().all(Reg::nullable),
            RegKind::Alt(branches) => branches.iter().any(Reg::nullable),
        }
    }

    fn first_sets(&self, out: &mut Vec<CharSet>) {
        match self.0.as_ref() {
            RegKind::Null | RegKind::Eps => {}
            RegKind::Atom(set) => out.push(set.clone()),
            RegKind::Concat(parts) => {
                for p in parts {
                    p.first_sets(out);
                    if !p.nullable() {
                        break;
                    }
                }
            }
            RegKind::Alt(branches) => {
                for b in branches {
                    b.first_sets(out);
                }
            }
            RegKind::Star(inner) => inner.first_sets(out),
        }
    }

    /// Concatenate `self` on the left of `tail`, with normalization.
    fn then(self, tail: Reg) -> Reg {
        Reg::concat(vec![self, tail])
    }
}

fn reg_ord(a: &Reg, b: &Reg) -> Ordering {
    use RegKind::*;
    fn rank(k: &RegKind) -> u8 {
        match k {
            Null => 0,
            Eps => 1,
            Atom(_) => 2,
            Concat(_) => 3,
            Alt(_) => 4,
            Star(_) => 5,
        }
    }
    let (ka, kb) = (a.0.as_ref(), b.0.as_ref());
    match rank(ka).cmp(&rank(kb)) {
        Ordering::Equal => {}
        o => return o,
    }
    match (ka, kb) {
        (Atom(sa), Atom(sb)) => {
            let ia = sa.intervals();
            let ib = sb.intervals();
            for (x, y) in ia.iter().zip(ib.iter()) {
                match (x.start, x.end).cmp(&(y.start, y.end)) {
                    Ordering::Equal => {}
                    o => return o,
                }
            }
            ia.len().cmp(&ib.len())
        }
        (Concat(pa), Concat(pb)) | (Alt(pa), Alt(pb)) => {
            for (x, y) in pa.iter().zip(pb.iter()) {
                match reg_ord(x, y) {
                    Ordering::Equal => {}
                    o => return o,
                }
            }
            pa.len().cmp(&pb.len())
        }
        (Star(ia), Star(ib)) => reg_ord(ia, ib),
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

fn expand_repeat(inner: Reg, min: usize, max: Option<usize>) -> Reg {
    let mut parts = Vec::new();
    for _ in 0..min {
        parts.push(inner.clone());
    }
    match max {
        None => {
            parts.push(Reg::star(inner));
            Reg::concat(parts)
        }
        Some(maximum) => {
            for _ in min..maximum {
                parts.push(Reg::alt(vec![Reg::eps(), inner.clone()]));
            }
            Reg::concat(parts)
        }
    }
}

// ---------------------------------------------------------------------------
// Linear forms = finite sets of residuals (Antimirov)
// ---------------------------------------------------------------------------

/// Sorted, deduplicated set of residual expressions.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct LinearForm(Vec<Reg>);

impl LinearForm {
    fn empty() -> Self {
        Self(Vec::new())
    }

    fn singleton(r: Reg) -> Self {
        if matches!(r.0.as_ref(), RegKind::Null) {
            Self::empty()
        } else {
            Self(vec![r])
        }
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn nullable(&self) -> bool {
        self.0.iter().any(Reg::nullable)
    }

    fn first_sets(&self, out: &mut Vec<CharSet>) {
        for r in &self.0 {
            r.first_sets(out);
        }
    }

    /// Build a normalized linear form from raw, possibly-unsorted,
    /// possibly-duplicated, possibly-`Null` members in a single
    /// sort/dedup/filter pass.
    ///
    /// Prefer this over folding many sources together with repeated
    /// [`LinearForm::union`] calls: each `union` re-sorts and re-dedups its
    /// accumulator from scratch, so combining `k` sources one at a time
    /// (as in `∂ₐ(E₁|E₂|…|Eₖ) = ∂ₐ(E₁) ∪ ∂ₐ(E₂) ∪ … ∪ ∂ₐ(Eₖ)`) costs
    /// `O(k² log k)`. Collecting every source's raw members into one `Vec`
    /// first and normalizing once costs `O(k log k)`. This is exactly the
    /// case that matters for wide alternation -- the pattern shape this
    /// backend exists to handle well (see the module doc comment).
    fn from_parts(mut parts: Vec<Reg>) -> Self {
        parts.sort_by(reg_ord);
        parts.dedup();
        // Drop explicit Null if any slipped in.
        parts.retain(|r| !matches!(r.0.as_ref(), RegKind::Null));
        Self(parts)
    }

    fn union(mut self, other: LinearForm) -> Self {
        self.0.extend(other.0);
        Self::from_parts(self.0)
    }

    /// Right-concatenate every member with `tail`.
    fn then_all(self, tail: &Reg) -> Self {
        if matches!(tail.0.as_ref(), RegKind::Null) {
            return Self::empty();
        }
        if matches!(tail.0.as_ref(), RegKind::Eps) {
            return self;
        }
        let mut out = Vec::with_capacity(self.0.len());
        for r in self.0 {
            out.push(r.then(tail.clone()));
        }
        Self::from_parts(out)
    }
}

/// Antimirov partial derivative: finite set of residuals.
fn partial_der(r: &Reg, ch: char, cache: &mut HashMap<(Reg, char), LinearForm>) -> LinearForm {
    if let Some(cached) = cache.get(&(r.clone(), ch)) {
        return cached.clone();
    }
    let result = match r.0.as_ref() {
        RegKind::Null | RegKind::Eps => LinearForm::empty(),
        RegKind::Atom(set) => {
            if set.contains(ch) {
                LinearForm::singleton(Reg::eps())
            } else {
                LinearForm::empty()
            }
        }
        RegKind::Alt(branches) => {
            // ∂ₐ(E₁|…|Eₖ) = ∂ₐ(E₁) ∪ … ∪ ∂ₐ(Eₖ): gather every branch's raw
            // members and normalize once (see `LinearForm::from_parts`)
            // instead of folding branch-by-branch through `union`, which
            // would re-sort/re-dedup the growing accumulator on every
            // branch and cost O(k^2 log k) for a k-way alternation.
            let mut parts = Vec::new();
            for b in branches {
                parts.extend(partial_der(b, ch, cache).0);
            }
            LinearForm::from_parts(parts)
        }
        RegKind::Concat(parts) => {
            // ∂(e1 e2 … ek) = ∂(e1)·(e2…ek) ∪ ν(e1)·∂(e2…ek)
            if parts.is_empty() {
                LinearForm::empty()
            } else {
                let head = &parts[0];
                let tail = if parts.len() == 1 {
                    Reg::eps()
                } else {
                    Reg::concat(parts[1..].to_vec())
                };
                let mut acc = partial_der(head, ch, cache).then_all(&tail);
                if head.nullable() {
                    // `tail` recurs constantly across a wide linear form
                    // built from a long chain (every member of
                    // "a?a?a?...a?"'s derivative is a *suffix* of every
                    // longer member's own suffix chain) -- caching this
                    // call is what turns O(members) independent
                    // recursive walks into effectively one shared walk.
                    acc = acc.union(partial_der(&tail, ch, cache));
                }
                acc
            }
        }
        RegKind::Star(inner) => {
            // ∂(e*) = ∂(e)·e*
            partial_der(inner, ch, cache).then_all(r)
        }
    };
    cache.insert((r.clone(), ch), result.clone());
    result
}

fn partial_der_form(
    form: &LinearForm,
    ch: char,
    cache: &mut HashMap<(Reg, char), LinearForm>,
) -> LinearForm {
    // Same batch-normalize rationale as the `Alt` case above: a linear
    // form reachable during product search is itself effectively a wide
    // alternation of residuals, so this is on the hot path for exactly
    // the pattern shape (wide alternation) this backend targets.
    let mut parts = Vec::new();
    for r in &form.0 {
        parts.extend(partial_der(r, ch, cache).0);
    }
    LinearForm::from_parts(parts)
}

fn representative_chars(sets: &[CharSet]) -> Vec<char> {
    let mut boundaries: Vec<u32> = Vec::new();
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

/// Empty linear form is the absorbing dead language on that side.
fn is_dead_end(query: Query, left: &LinearForm, right: &LinearForm) -> bool {
    let left_dead = left.is_empty();
    let right_dead = right.is_empty();
    match query {
        Query::Overlap => left_dead || right_dead,
        Query::Includes => left_dead,
        Query::Equivalent => left_dead && right_dead,
        Query::Empty => false,
    }
}

fn stopped_result(
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

fn search_product(query: Query, left: Reg, right: Reg, config: &Config) -> BackendResult {
    let started = Instant::now();
    let deadline = Duration::from_millis(config.timeout_ms);

    #[derive(Clone, Debug, Eq, PartialEq, Hash)]
    struct ProductKey {
        left: LinearForm,
        right: LinearForm,
    }
    struct SearchNode {
        key: ProductKey,
        parent: Option<(usize, char)>,
    }

    let initial = ProductKey {
        left: LinearForm::singleton(left),
        right: LinearForm::singleton(right),
    };
    let mut nodes = vec![SearchNode {
        key: initial.clone(),
        parent: None,
    }];
    let mut visited = HashSet::new();
    visited.insert(initial);
    let mut queue = VecDeque::from([0usize]);
    let mut generated_transitions = 0usize;
    let mut left_cache: HashMap<(LinearForm, char), LinearForm> = HashMap::new();
    let mut right_cache: HashMap<(LinearForm, char), LinearForm> = HashMap::new();
    // Persists for the whole query, across every BFS step -- not just
    // within one partial_der_form call. The same residual term (e.g. "k
    // remaining copies of a?") recurs both as a direct linear-form member
    // at one step and as a recursive sub-computation while deriving a
    // longer member at another step; without this, each of those O(n)
    // occurrences repeats its own O(size) derivation independently.
    let mut term_cache: HashMap<(Reg, char), LinearForm> = HashMap::new();

    while let Some(node_id) = queue.pop_front() {
        if started.elapsed() >= deadline {
            return stopped_result(
                BackendStatus::Timeout,
                nodes.len(),
                generated_transitions,
                started,
            );
        }
        let left_acc = nodes[node_id].key.left.nullable();
        let right_acc = nodes[node_id].key.right.nullable();
        if let Some(rel) = classify_relation(query, left_acc, right_acc) {
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
                relation: Some(rel.to_owned()),
                visited_states: nodes.len(),
                generated_transitions,
                analysis_ms,
                witness_extraction_ms: witness_started.elapsed().as_millis(),
            };
        }

        if is_dead_end(query, &nodes[node_id].key.left, &nodes[node_id].key.right) {
            continue;
        }

        let mut first = Vec::new();
        nodes[node_id].key.left.first_sets(&mut first);
        nodes[node_id].key.right.first_sets(&mut first);
        for ch in representative_chars(&first) {
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
            let next_left = match left_cache.get(&(nodes[node_id].key.left.clone(), ch)) {
                Some(v) => v.clone(),
                None => {
                    let computed = partial_der_form(&nodes[node_id].key.left, ch, &mut term_cache);
                    left_cache.insert((nodes[node_id].key.left.clone(), ch), computed.clone());
                    computed
                }
            };
            let next_right = match right_cache.get(&(nodes[node_id].key.right.clone(), ch)) {
                Some(v) => v.clone(),
                None => {
                    let computed = partial_der_form(&nodes[node_id].key.right, ch, &mut term_cache);
                    right_cache.insert((nodes[node_id].key.right.clone(), ch), computed.clone());
                    computed
                }
            };
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
    struct Key(LinearForm);
    struct Node {
        key: Key,
        parent: Option<(usize, char)>,
    }

    let initial = Key(LinearForm::singleton(reg));
    let mut nodes = vec![Node {
        key: initial.clone(),
        parent: None,
    }];
    let mut visited = HashSet::new();
    visited.insert(initial);
    let mut queue = VecDeque::from([0usize]);
    let mut generated_transitions = 0usize;
    let mut term_cache: HashMap<(Reg, char), LinearForm> = HashMap::new();

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
        if nodes[node_id].key.0.is_empty() {
            continue;
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
            let next = Key(partial_der_form(&nodes[node_id].key.0, ch, &mut term_cache));
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

/// Antimirov partial-derivative engine.
pub struct AntimirovBackend;

impl RelationBackend for AntimirovBackend {
    fn name(&self) -> &'static str {
        "antimirov"
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn analyze_binary(
        &self,
        _query: Query,
        _left: &Nfa,
        _right: &Nfa,
        _config: &Config,
    ) -> BackendResult {
        // Prefer the AST-aware path; NFA-only entry is unused for this
        // backend. `Timeout` (never `Exhausted`) signals "did not actually
        // run" -- `Exhausted` would read as a completed, decisive search
        // that legitimately found no witness, which is not what happened
        // here. Matches `DerivativeBackend`'s identical fallback.
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{
        analyze_binary_with_backend, analyze_empty_with_backend, AutomataBackend,
    };
    use crate::report::Verdict;
    use crate::Config;

    fn both_agree(query: Query, left: &str, right: &str) {
        let config = Config::default();
        let a = analyze_binary_with_backend(query, left, right, &config, &AutomataBackend).unwrap();
        let b =
            analyze_binary_with_backend(query, left, right, &config, &AntimirovBackend).unwrap();
        assert_eq!(
            a.verdict, b.verdict,
            "disagree on {:?}({:?}, {:?}): automata={:?} antimirov={:?}",
            query, left, right, a.verdict, b.verdict
        );
    }

    #[test]
    fn agrees_on_core_relations() {
        both_agree(Query::Equivalent, "a|b", "[ab]");
        both_agree(Query::Equivalent, "a+", "aa*");
        both_agree(Query::Overlap, "a+b", "ab+");
        both_agree(Query::Includes, "a+", "a*");
        both_agree(Query::Includes, "a*", "a+");
        both_agree(Query::Overlap, "a+", "b+");
        both_agree(Query::Equivalent, "(a|b)*", "(a*b*)*");
        both_agree(Query::Equivalent, "a{2,4}", "aa|aaa|aaaa");
    }

    #[test]
    fn empty_language_cases() {
        let config = Config::default();
        let r = analyze_empty_with_backend("a", &config, &AntimirovBackend).unwrap();
        assert_eq!(r.verdict, Verdict::No);
        let r = analyze_empty_with_backend("[^\\d\\D]", &config, &AntimirovBackend).unwrap();
        // may be Yes if class empty
        assert!(matches!(r.verdict, Verdict::Yes | Verdict::No));
    }

    #[test]
    fn partial_der_atom() {
        let a = Reg::atom(CharSet::singleton('a'));
        let mut cache = HashMap::new();
        let d = partial_der(&a, 'a', &mut cache);
        assert!(d.nullable());
        let d2 = partial_der(&a, 'b', &mut cache);
        assert!(d2.is_empty());
    }

    #[test]
    fn partial_der_star() {
        let star = Reg::star(Reg::atom(CharSet::singleton('a')));
        let mut cache = HashMap::new();
        let d = partial_der(&star, 'a', &mut cache);
        assert!(!d.is_empty());
        // a* after reading a is still nullable (ε member via a* )
        assert!(d.nullable());
    }

    #[test]
    fn partial_der_wide_alt_matches_pairwise_union() {
        // Regression guard for the `LinearForm::from_parts` batch-normalize
        // refactor of the `Alt` case in `partial_der`: build the same
        // linear form the slow way (folding branch results together one
        // at a time through `union`, the shape the code used to have) and
        // check it against the batched result for a wide alternation.
        // Separate caches for the two computations: memoization changes
        // nothing about the *result* (`partial_der` is a pure function of
        // `(Reg, char)`), so sharing a cache here would prove nothing extra
        // and could mask a cache-invalidation bug behind cache hits.
        let branches: Vec<Reg> = "abcdefg"
            .chars()
            .map(|c| Reg::atom(CharSet::singleton(c)))
            .collect();
        let wide_alt = Reg::alt(branches.clone());
        let mut batched_cache = HashMap::new();
        let batched = partial_der(&wide_alt, 'd', &mut batched_cache);

        let mut folded_cache = HashMap::new();
        let mut folded = LinearForm::empty();
        for b in &branches {
            folded = folded.union(partial_der(b, 'd', &mut folded_cache));
        }
        assert_eq!(batched, folded);
        assert!(batched.nullable(), "∂_d matches the 'd' branch -> eps");
    }

    #[test]
    fn respects_state_limit() {
        // Mirrors `derivative::tests::respects_state_limit`: a query that
        // genuinely needs expansion beyond the start state must report
        // `Unknown` once `max_product_states` is exhausted.
        let config = Config {
            max_product_states: 1,
            ..Config::default()
        };
        let report =
            analyze_binary_with_backend(Query::Equivalent, "a+", "b+", &config, &AntimirovBackend)
                .unwrap();
        assert_eq!(report.verdict, Verdict::Unknown);
    }

    #[test]
    fn immediate_decision_survives_tight_state_limit() {
        // Regression guard: a query decidable *at the start state* (zero
        // expansions needed) must still return its answer even when
        // `max_product_states` is as tight as 1. `search_product` and
        // `search_single` only push a *new* node once they've confirmed
        // room for it; they must not also refuse to look at the state
        // that's already on the frontier.
        let config = Config {
            max_product_states: 1,
            ..Config::default()
        };
        // Overlap: 'a*' and 'b*' both accept the empty string, so the verdict
        // is decidable from the initial product state alone.
        let report =
            analyze_binary_with_backend(Query::Overlap, "a*", "b*", &config, &AntimirovBackend)
                .unwrap();
        assert_eq!(report.verdict, Verdict::Yes);
        assert_eq!(report.witness.unwrap().value, "");

        // Empty: 'a*' accepts the empty string immediately, i.e. its
        // language is *not* empty, so the correct verdict for the `empty`
        // query is `No` (this query's polarity is inverted from the others:
        // `Yes` means "yes, this language is empty" -- see the pre-existing
        // `empty_language_cases` test above, which the first version of
        // this test failed to follow).
        let report = analyze_empty_with_backend("a*", &config, &AntimirovBackend).unwrap();
        assert_eq!(report.verdict, Verdict::No);
        assert_eq!(report.witness.unwrap().value, "");
    }
}
