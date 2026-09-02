use crate::ast::{Expr, ExprKind};
use crate::charset::CharSet;

#[derive(Clone, Debug)]
pub struct Transition {
    pub set: CharSet,
    pub target: usize,
}

#[derive(Clone, Debug, Default)]
pub struct State {
    pub epsilon: Vec<usize>,
    pub transitions: Vec<Transition>,
}

#[derive(Clone, Debug)]
pub struct Nfa {
    pub states: Vec<State>,
    pub start: usize,
    pub accept: usize,
}

#[derive(Clone, Copy, Debug)]
struct Fragment {
    start: usize,
    end: usize,
}

impl Nfa {
    pub fn from_expr(expr: &Expr) -> Self {
        let mut builder = Builder { states: Vec::new() };
        let fragment = builder.build(expr);
        Self {
            states: builder.states,
            start: fragment.start,
            accept: fragment.end,
        }
    }

    pub fn start_subset(&self) -> Vec<usize> {
        self.epsilon_closure([self.start])
    }

    pub fn step(&self, subset: &[usize], ch: char) -> Vec<usize> {
        let mut seen = vec![false; self.states.len()];
        let mut targets = Vec::new();
        for state_id in subset {
            for transition in &self.states[*state_id].transitions {
                if transition.set.contains(ch) && !seen[transition.target] {
                    seen[transition.target] = true;
                    targets.push(transition.target);
                }
            }
        }
        self.epsilon_closure(targets)
    }

    /// Return whether a canonical NFA subset contains the accept state.
    ///
    /// Subsets produced by this type are sorted in ascending state-ID order.
    pub fn is_accepting(&self, subset: &[usize]) -> bool {
        debug_assert!(
            subset.windows(2).all(|window| window[0] < window[1]),
            "NFA subsets must be sorted and duplicate-free"
        );
        subset.binary_search(&self.accept).is_ok()
    }

    pub fn outgoing_sets<'a>(&'a self, subset: &'a [usize]) -> impl Iterator<Item = &'a CharSet> {
        subset.iter().flat_map(move |state_id| {
            self.states[*state_id]
                .transitions
                .iter()
                .map(|transition| &transition.set)
        })
    }

    pub fn matches(&self, input: &str) -> bool {
        let mut subset = self.start_subset();
        for ch in input.chars() {
            subset = self.step(&subset, ch);
            if subset.is_empty() {
                return false;
            }
        }
        self.is_accepting(&subset)
    }

    fn epsilon_closure<I>(&self, initial: I) -> Vec<usize>
    where
        I: IntoIterator<Item = usize>,
    {
        let mut visited = vec![false; self.states.len()];
        let mut stack = Vec::new();
        for state in initial {
            if !visited[state] {
                visited[state] = true;
                stack.push(state);
            }
        }
        while let Some(state) = stack.pop() {
            for target in &self.states[state].epsilon {
                if !visited[*target] {
                    visited[*target] = true;
                    stack.push(*target);
                }
            }
        }
        visited
            .into_iter()
            .enumerate()
            .filter_map(|(state, included)| included.then_some(state))
            .collect()
    }
}

struct Builder {
    states: Vec<State>,
}

impl Builder {
    fn state(&mut self) -> usize {
        let id = self.states.len();
        self.states.push(State::default());
        id
    }

    fn epsilon(&mut self, from: usize, to: usize) {
        self.states[from].epsilon.push(to);
    }

    fn transition(&mut self, from: usize, to: usize, set: CharSet) {
        if !set.is_empty() {
            self.states[from]
                .transitions
                .push(Transition { set, target: to });
        }
    }

    fn empty(&mut self) -> Fragment {
        let start = self.state();
        let end = self.state();
        self.epsilon(start, end);
        Fragment { start, end }
    }

    fn build(&mut self, expr: &Expr) -> Fragment {
        match &expr.kind {
            ExprKind::Empty | ExprKind::AnchorStart | ExprKind::AnchorEnd => self.empty(),
            ExprKind::Literal(ch) => {
                let start = self.state();
                let end = self.state();
                self.transition(start, end, CharSet::singleton(*ch));
                Fragment { start, end }
            }
            ExprKind::CharSet(set) => {
                let start = self.state();
                let end = self.state();
                self.transition(start, end, set.clone());
                Fragment { start, end }
            }
            ExprKind::Concat(parts) => {
                let mut iter = parts.iter();
                let Some(first) = iter.next() else {
                    return self.empty();
                };
                let mut result = self.build(first);
                for part in iter {
                    let next = self.build(part);
                    self.epsilon(result.end, next.start);
                    result.end = next.end;
                }
                result
            }
            ExprKind::Alt(branches) => {
                let start = self.state();
                let end = self.state();
                // An alternation with zero branches denotes the empty
                // language (the identity element for union), matching
                // `Reg::alt`'s treatment of an empty branch list in
                // `derivative.rs` and the classic Kleene-algebra convention
                // that a sum over an empty index set is the additive
                // identity `∅`. Leaving `start`/`end` unconnected already
                // encodes `∅` correctly here (the same pattern `transition`
                // uses for an empty `CharSet`, just above), so there is
                // nothing to add for this case: adding an epsilon edge would
                // wrongly make the fragment accept `{ε}` instead.
                //
                // The parser never constructs `ExprKind::Alt(vec![])`
                // (`parse_alt` always starts `branches` with one parsed
                // element, and unwraps to that single branch instead of
                // wrapping it in `Alt` when no `|` follows), so this only
                // matters for `Expr` trees built directly through the
                // public AST types rather than through `parse`.
                for branch in branches {
                    let fragment = self.build(branch);
                    self.epsilon(start, fragment.start);
                    self.epsilon(fragment.end, end);
                }
                Fragment { start, end }
            }
            ExprKind::Repeat { expr, min, max } => self.repeat(expr, *min, *max),
        }
    }

    fn repeat(&mut self, expr: &Expr, min: usize, max: Option<usize>) -> Fragment {
        let mut result = self.empty();
        // Track the last required copy's fragment (if any were built) so an
        // unbounded tail (`max == None`, below) can loop directly on it
        // instead of building yet another, otherwise-redundant copy of
        // `expr` purely to serve as the star body.
        let mut last_required: Option<Fragment> = None;
        for _ in 0..min {
            let copy = self.build(expr);
            self.epsilon(result.end, copy.start);
            result.end = copy.end;
            last_required = Some(copy);
        }

        match max {
            Some(maximum) => {
                for _ in min..maximum {
                    let optional_end = self.state();
                    let copy = self.build(expr);
                    self.epsilon(result.end, optional_end);
                    self.epsilon(result.end, copy.start);
                    self.epsilon(copy.end, optional_end);
                    result.end = optional_end;
                }
                result
            }
            None => {
                let loop_end = self.state();
                match last_required {
                    // `a{m,}` with m >= 1: the last required copy already
                    // sits at `result.end`/`result.start` of its own
                    // fragment -- reuse it as the loop body directly
                    // (Thompson's usual `a+` shape: one fragment, plus a
                    // back-edge from its end to its own start) rather than
                    // building an (m+1)-th copy whose only job would be to
                    // be identical to the one already sitting right here.
                    Some(copy) => {
                        self.epsilon(result.end, loop_end);
                        self.epsilon(copy.end, copy.start);
                        result.end = loop_end;
                        result
                    }
                    // `a*` (m == 0): no required copy exists to reuse, so
                    // build the one and only copy the star needs -- already
                    // minimal, unchanged from before.
                    None => {
                        let copy = self.build(expr);
                        self.epsilon(result.end, loop_end);
                        self.epsilon(result.end, copy.start);
                        self.epsilon(copy.end, loop_end);
                        self.epsilon(copy.end, copy.start);
                        result.end = loop_end;
                        result
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Span;
    use crate::{parse, Alphabet, Config};

    fn compile(pattern: &str) -> Nfa {
        Nfa::from_expr(&parse(pattern, &Config::default()).unwrap())
    }

    fn compile_with(pattern: &str, config: &Config) -> Nfa {
        Nfa::from_expr(&parse(pattern, config).unwrap())
    }

    #[test]
    fn concatenation_requires_every_part() {
        let nfa = compile("abc");
        assert!(nfa.matches("abc"));
        assert!(!nfa.matches("ab"));
        assert!(!nfa.matches("abcd"));
    }

    #[test]
    fn alternation_accepts_either_branch() {
        let nfa = compile("ab|cd");
        assert!(nfa.matches("ab"));
        assert!(nfa.matches("cd"));
        assert!(!nfa.matches("ad"));
    }

    #[test]
    fn repeat_matches_expected_language() {
        let nfa = compile("ab{2,3}");
        assert!(nfa.matches("abb"));
        assert!(nfa.matches("abbb"));
        assert!(!nfa.matches("ab"));
        assert!(!nfa.matches("abbbb"));
    }

    #[test]
    fn exact_zero_repeat_accepts_only_empty() {
        let nfa = compile("a{0}");
        assert!(nfa.matches(""));
        assert!(!nfa.matches("a"));
    }

    #[test]
    fn empty_alternation_denotes_the_empty_language() {
        // `parse` can never produce `ExprKind::Alt(vec![])` -- `parse_alt`
        // always seeds `branches` with one parsed element and unwraps to it
        // directly instead of wrapping a lone branch in `Alt` -- and
        // external construction of `Expr`/`ExprKind` is crate-private
        // (`ExprKind` is also `#[non_exhaustive]`) specifically so a
        // library caller can't hand one in either. So this exercises
        // `Nfa::from_expr` purely as an internal invariant check: nothing
        // downstream should choke on an empty alternation if some future
        // internal rewrite pass ever produces one. An empty alternation is
        // the identity element for union, so it must denote `∅` (matching
        // nothing, not even the empty string) -- see `residual.rs`'s
        // equivalent `from_expr`, which reaches the same `RegKind::Null`
        // conclusion for the shared symbolic backends.
        let expr = Expr::new(ExprKind::Alt(Vec::new()), Span::new(0, 0));
        let nfa = Nfa::from_expr(&expr);
        assert!(!nfa.matches(""));
        assert!(!nfa.matches("a"));
    }

    /// Regression test for the Thompson construction of `a{m,}`/`a+`
    /// (`m >= 1`, unbounded): `repeat`'s `max == None` arm used to always
    /// build a fresh `(m+1)`-th copy of the body purely to serve as the
    /// star tail, even though the `m`-th (last required) copy it had just
    /// finished building was already sitting right there, unused for
    /// anything but feeding into the next repetition. Looping directly on
    /// that last required copy instead should build a strictly smaller
    /// NFA -- exactly one body-sized fragment fewer -- while matching the
    /// identical language.
    #[test]
    fn unbounded_repeat_with_required_minimum_reuses_the_last_copy() {
        let nfa = compile("a{3,}");
        // 3 required copies of 'a' (2 states each, 6 total) + the shared
        // `self.empty()` fragment feeding the first one (2 states) + one
        // `loop_end` state for the unbounded tail = 9. Before this fix, an
        // extra (4th) copy of 'a' built purely for the star tail made this
        // 11 -- the exact count is a stronger, more direct check than
        // comparing against a different pattern's count, and pins the
        // savings to this specific, previously-measured before/after pair
        // rather than an indirect relationship that could hold by
        // coincidence.
        assert_eq!(nfa.states.len(), 9);

        assert!(!nfa.matches("aa"));
        assert!(nfa.matches("aaa"));
        assert!(nfa.matches("aaaaaaaa"));
    }

    #[test]
    fn unbounded_repeat_with_no_minimum_is_unaffected() {
        // `a*` (min == 0) has no required copy to reuse, so its
        // construction -- and state count -- must be exactly what it was
        // before this fix: the shared `self.empty()` fragment (2 states) +
        // one copy of the body (2 states) + `loop_end` (1 state) = 5.
        let nfa = compile("a*");
        assert_eq!(nfa.states.len(), 5);
        assert!(nfa.matches(""));
        assert!(nfa.matches("aaaaa"));
    }

    #[test]
    fn star_accepts_empty_and_multiple_copies() {
        let nfa = compile("(ab)*");
        assert!(nfa.matches(""));
        assert!(nfa.matches("ab"));
        assert!(nfa.matches("abab"));
        assert!(!nfa.matches("aba"));
    }

    #[test]
    fn plus_requires_one_copy() {
        let nfa = compile("a+");
        assert!(!nfa.matches(""));
        assert!(nfa.matches("a"));
        assert!(nfa.matches("aaa"));
    }

    #[test]
    fn optional_accepts_zero_or_one_copy() {
        let nfa = compile("colou?r");
        assert!(nfa.matches("color"));
        assert!(nfa.matches("colour"));
        assert!(!nfa.matches("colouur"));
    }

    #[test]
    fn classes_and_negated_classes_use_the_configured_alphabet() {
        let positive = compile("[a-c]");
        assert!(positive.matches("b"));
        assert!(!positive.matches("d"));

        let negative = compile("[^a]");
        assert!(negative.matches("b"));
        assert!(!negative.matches("a"));
        assert!(!negative.matches("é"));
    }

    #[test]
    fn shorthand_classes_match_documented_ascii_sets() {
        assert!(compile(r"\d").matches("5"));
        assert!(!compile(r"\d").matches("x"));
        assert!(compile(r"\w").matches("_"));
        assert!(compile(r"\s").matches("\t"));
    }

    #[test]
    fn dot_newline_behavior_is_configurable() {
        assert!(!compile(".").matches("\n"));
        let config = Config {
            dot_matches_newline: true,
            ..Config::default()
        };
        assert!(compile_with(".", &config).matches("\n"));
    }

    #[test]
    fn outer_anchors_do_not_change_full_string_language() {
        let anchored = compile("^ab$");
        let plain = compile("ab");
        for word in ["", "a", "ab", "zab", "abz"] {
            assert_eq!(anchored.matches(word), plain.matches(word));
        }
    }

    #[test]
    fn unicode_literal_matches_in_unicode_mode() {
        let config = Config {
            alphabet: Alphabet::Unicode,
            ..Config::default()
        };
        let nfa = compile_with("é+", &config);
        assert!(nfa.matches("é"));
        assert!(nfa.matches("éé"));
        assert!(!nfa.matches("e"));
    }
}
