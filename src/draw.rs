//! Render a regex as a Graphviz automaton (NFA, DFA, or minimized DFA).
//!
//! Construction reuses the crate's Thompson NFA (`nfa::Nfa::from_expr`) and
//! the existing subset-construction / Moore-minimization pipeline in
//! `minimize`. The DOT is then handed to Graphviz `dot` to produce a PDF.

use crate::analysis::BackendStatus;
use crate::charset::CharSet;
use crate::config::Config;
use crate::minimize::{determinize, minimize, Dfa, DEAD};
use crate::nfa::Nfa;
use crate::parser::{parse, FrontendError};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DrawKind {
    Nfa,
    Dfa,
    Minimized,
}

impl DrawKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Nfa => "nfa",
            Self::Dfa => "dfa",
            Self::Minimized => "minimized",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Nfa => "NFA",
            Self::Dfa => "DFA",
            Self::Minimized => "Minimized DFA",
        }
    }
}

#[derive(Debug, Error)]
pub enum DrawError {
    #[error(transparent)]
    Input(#[from] FrontendError),
    #[error("{0}")]
    Limit(String),
    #[error("failed to write {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "graphviz `dot` was not found on PATH; install Graphviz, or pass --emit-dot to print DOT"
    )]
    DotMissing,
    #[error("graphviz `dot` failed{code}: {stderr}")]
    DotFailed { code: String, stderr: String },
}

#[derive(Clone, Debug)]
pub struct DrawResult {
    pub kind: DrawKind,
    pub pattern: String,
    pub dot: String,
    pub state_count: usize,
    pub transition_count: usize,
}

/// Parse `pattern`, build the requested automaton, and return Graphviz DOT.
pub fn draw_dot(pattern: &str, kind: DrawKind, config: &Config) -> Result<DrawResult, DrawError> {
    let expr = parse(pattern, config)?;
    let nfa = Nfa::from_expr(&expr);
    let (dot, state_count, transition_count) = match kind {
        DrawKind::Nfa => nfa_to_dot(&nfa, pattern, config),
        DrawKind::Dfa => {
            let dfa = determinize_for_draw(&nfa, config)?;
            dfa_to_dot(&dfa, pattern, kind, config)
        }
        DrawKind::Minimized => {
            let dfa = determinize_for_draw(&nfa, config)?;
            let minimized = minimize_for_draw(&dfa, config)?;
            dfa_to_dot(&minimized, pattern, kind, config)
        }
    };
    Ok(DrawResult {
        kind,
        pattern: pattern.to_owned(),
        dot,
        state_count,
        transition_count,
    })
}

/// Write `dot` through Graphviz `dot` to `output`.
///
/// The output format is taken from the file extension: `.pdf` (default),
/// `.svg`, `.png`, or `.dot` (the source is written directly, `dot` is not
/// invoked).
pub fn render_graph(dot: &str, output: &Path) -> Result<(), DrawError> {
    let ext = output
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("pdf")
        .to_ascii_lowercase();
    if ext == "dot" {
        return std::fs::write(output, dot).map_err(|source| DrawError::Io {
            path: output.display().to_string(),
            source,
        });
    }
    let format = match ext.as_str() {
        "svg" => "svg",
        "png" => "png",
        "ps" => "ps",
        _ => "pdf",
    };
    let dot_bin = find_dot().ok_or(DrawError::DotMissing)?;
    let mut child = Command::new(&dot_bin)
        .arg(format!("-T{format}"))
        .arg("-o")
        .arg(output)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                DrawError::DotMissing
            } else {
                DrawError::Io {
                    path: dot_bin.display().to_string(),
                    source,
                }
            }
        })?;
    {
        let stdin = child.stdin.as_mut().expect("piped stdin");
        stdin
            .write_all(dot.as_bytes())
            .map_err(|source| DrawError::Io {
                path: "dot stdin".to_owned(),
                source,
            })?;
    }
    let finished = child.wait_with_output().map_err(|source| DrawError::Io {
        path: dot_bin.display().to_string(),
        source,
    })?;
    if !finished.status.success() {
        let stderr = String::from_utf8_lossy(&finished.stderr).trim().to_owned();
        let code = finished
            .status
            .code()
            .map(|c| format!(" (exit {c})"))
            .unwrap_or_default();
        return Err(DrawError::DotFailed {
            code,
            stderr: if stderr.is_empty() {
                "no diagnostic from dot".to_owned()
            } else {
                stderr
            },
        });
    }
    Ok(())
}

fn determinize_for_draw(nfa: &Nfa, config: &Config) -> Result<Dfa, DrawError> {
    let started = Instant::now();
    let deadline = Duration::from_millis(config.timeout_ms);
    determinize(nfa, config.max_product_states, started, deadline).map_err(|status| {
        DrawError::Limit(limit_message(
            status,
            "determinization",
            config.max_product_states,
            config.timeout_ms,
        ))
    })
}

fn minimize_for_draw(dfa: &Dfa, config: &Config) -> Result<Dfa, DrawError> {
    let started = Instant::now();
    let deadline = Duration::from_millis(config.timeout_ms);
    minimize(dfa, started, deadline).map_err(|status| {
        DrawError::Limit(limit_message(
            status,
            "minimization",
            config.max_product_states,
            config.timeout_ms,
        ))
    })
}

fn limit_message(status: BackendStatus, phase: &str, max_states: usize, timeout_ms: u64) -> String {
    match status {
        BackendStatus::StateLimit => format!(
            "{phase} hit the state limit ({max_states}); raise --max-states or simplify the pattern"
        ),
        BackendStatus::Timeout => format!(
            "{phase} hit the time limit ({timeout_ms} ms); raise --timeout-ms or simplify the pattern"
        ),
        other => format!("{phase} failed ({other:?})"),
    }
}

fn find_dot() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("REGEXREL_DOT") {
        let path = PathBuf::from(explicit);
        if path.is_file() {
            return Some(path);
        }
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join("dot");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn nfa_to_dot(nfa: &Nfa, pattern: &str, config: &Config) -> (String, usize, usize) {
    let mut labeled: BTreeMap<(usize, usize), CharSet> = BTreeMap::new();
    let mut epsilons: Vec<(usize, usize)> = Vec::new();
    for (from, state) in nfa.states.iter().enumerate() {
        for &to in &state.epsilon {
            epsilons.push((from, to));
        }
        for transition in &state.transitions {
            labeled
                .entry((from, transition.target))
                .and_modify(|set| *set = set.union(&transition.set))
                .or_insert_with(|| transition.set.clone());
        }
    }
    let mut edges = Vec::new();
    for (from, to) in epsilons {
        edges.push(dot_edge(from, to, "ε"));
    }
    for ((from, to), set) in &labeled {
        let label = set.to_dot_label(config.alphabet, config.dot_matches_newline);
        edges.push(dot_edge(*from, *to, &label));
    }
    let transition_count = nfa
        .states
        .iter()
        .map(|s| s.epsilon.len() + s.transitions.len())
        .sum();
    let body = automaton_dot(
        DrawKind::Nfa,
        pattern,
        nfa.states.len(),
        nfa.start,
        |id| id == nfa.accept,
        |_| None,
        &edges,
    );
    (body, nfa.states.len(), transition_count)
}

fn dfa_to_dot(dfa: &Dfa, pattern: &str, kind: DrawKind, config: &Config) -> (String, usize, usize) {
    let skip_dead = dfa.start != DEAD;
    let visible: Vec<usize> = dfa
        .states
        .iter()
        .enumerate()
        .filter(|(id, _)| !(skip_dead && *id == DEAD))
        .map(|(id, _)| id)
        .collect();
    let mut labeled: BTreeMap<(usize, usize), CharSet> = BTreeMap::new();
    for &from in &visible {
        for (set, to) in &dfa.states[from].transitions {
            if skip_dead && *to == DEAD {
                continue;
            }
            labeled
                .entry((from, *to))
                .and_modify(|existing| *existing = existing.union(set))
                .or_insert_with(|| set.clone());
        }
    }
    let mut edges = Vec::new();
    for ((from, to), set) in &labeled {
        let label = set.to_dot_label(config.alphabet, config.dot_matches_newline);
        edges.push(dot_edge(*from, *to, &label));
    }
    let body = automaton_dot(
        kind,
        pattern,
        dfa.states.len(),
        dfa.start,
        |id| dfa.states[id].accepting,
        |id| {
            let members = &dfa.states[id].members;
            Some(format_subset(members))
        },
        &edges,
    );
    let state_count = visible.len();
    (body, state_count, labeled.len())
}

fn format_subset(members: &[usize]) -> String {
    if members.is_empty() {
        return "∅".to_owned();
    }
    let inner = members
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    format!("{{{inner}}}")
}

fn automaton_dot(
    kind: DrawKind,
    pattern: &str,
    n_states: usize,
    start: usize,
    is_accept: impl Fn(usize) -> bool,
    node_label: impl Fn(usize) -> Option<String>,
    edges: &[String],
) -> String {
    let mut out = String::new();
    out.push_str("digraph {\n");
    out.push_str("  graph [rankdir=LR, pad=\"0.3\", bgcolor=\"transparent\"];\n");
    out.push_str("  node [shape=circle, fontsize=12, fontname=\"Helvetica\"];\n");
    out.push_str("  edge [fontsize=11, fontname=\"Helvetica\", arrowsize=0.7];\n");
    let caption = format!("{} of {}", kind.title(), pattern);
    out.push_str(&format!(
        "  labelloc=\"t\";\n  label={};\n",
        quote_dot(&caption)
    ));
    out.push_str("  __start [shape=point, width=0.12, height=0.12, label=\"\"];\n");
    out.push_str(&format!("  __start -> q{start};\n"));
    for id in 0..n_states {
        if kind != DrawKind::Nfa && id == DEAD && start != DEAD {
            continue;
        }
        let mut attrs = Vec::new();
        if is_accept(id) {
            attrs.push("shape=doublecircle".to_owned());
        }
        if let Some(label) = node_label(id) {
            attrs.push(format!("label={}", quote_dot(&label)));
        }
        if attrs.is_empty() {
            out.push_str(&format!("  q{id};\n"));
        } else {
            out.push_str(&format!("  q{id} [{}];\n", attrs.join(", ")));
        }
    }
    for edge in edges {
        out.push_str(edge);
    }
    out.push_str("}\n");
    out
}

fn dot_edge(from: usize, to: usize, label: &str) -> String {
    format!("  q{from} -> q{to} [label={}];\n", quote_dot(label))
}

fn quote_dot(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(pattern: &str, kind: DrawKind) -> DrawResult {
        draw_dot(pattern, kind, &Config::default()).unwrap()
    }

    #[test]
    fn nfa_of_a_literal_has_a_labeled_edge() {
        let result = draw("a", DrawKind::Nfa);
        assert!(result.dot.contains("digraph"));
        assert!(result.dot.contains("label=\"a\""));
        assert!(result.dot.contains("shape=doublecircle"));
        assert!(result.dot.contains("__start -> q"));
        assert!(result.state_count >= 2);
        assert!(result.transition_count >= 1);
    }

    #[test]
    fn nfa_of_star_has_epsilon_edges() {
        let result = draw("a*", DrawKind::Nfa);
        assert!(result.dot.contains("label=\"ε\""));
        assert!(result.dot.contains("label=\"a\""));
    }

    #[test]
    fn dfa_has_no_epsilon_and_is_labeled_with_subsets() {
        let result = draw("a|b", DrawKind::Dfa);
        assert!(!result.dot.contains("label=\"ε\""));
        assert!(result.dot.contains("label=\"a\""));
        assert!(result.dot.contains("label=\"b\""));
        assert!(result.dot.contains('{') && result.dot.contains('}'));
    }

    #[test]
    fn minimized_dfa_collapses_equivalent_patterns() {
        let left = draw("a|b", DrawKind::Minimized);
        let right = draw("[ab]", DrawKind::Minimized);
        assert_eq!(left.state_count, right.state_count);
        assert!(left.state_count >= 2);
        assert!(left.state_count <= 3);
    }

    #[test]
    fn minimized_is_no_larger_than_unminimized_dfa() {
        let dfa = draw("(a|b)*a", DrawKind::Dfa);
        let min = draw("(a|b)*a", DrawKind::Minimized);
        assert!(min.state_count <= dfa.state_count);
    }

    #[test]
    fn class_and_dot_labels_stay_compact() {
        let digits = draw(r"\d", DrawKind::Nfa);
        assert!(digits.dot.contains("label=\"\\\\d\"") || digits.dot.contains(r#"label="\d""#));
        let any = draw(".", DrawKind::Nfa);
        assert!(any.dot.contains("label=\".\""));
    }

    #[test]
    fn syntax_error_is_surfaced() {
        let error = draw_dot("(", DrawKind::Nfa, &Config::default()).unwrap_err();
        assert!(matches!(error, DrawError::Input(_)));
    }

    #[test]
    fn empty_language_still_produces_a_graph() {
        let result = draw(r"[^\d\D]", DrawKind::Dfa);
        assert!(result.dot.contains("digraph"));
        assert!(result.state_count >= 1);
    }

    #[test]
    fn quote_dot_escapes_specials() {
        assert_eq!(quote_dot(r#"a"b\"#), r#""a\"b\\""#);
    }

    #[test]
    fn render_graph_writes_dot_without_invoking_graphviz() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("regexrel-draw-test-{}.dot", std::process::id()));
        let result = draw("ab", DrawKind::Nfa);
        render_graph(&result.dot, &path).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(written, result.dot);
    }
}
