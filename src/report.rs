use crate::analysis::Query;
use crate::config::Alphabet;
use crate::parser::FrontendError;
use serde::Serialize;

pub mod relation {
    pub const IN_BOTH: &str = "in_both";
    pub const LEFT_ONLY: &str = "left_only";
    pub const RIGHT_ONLY: &str = "right_only";
    pub const IN_LANGUAGE: &str = "in_language";
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Verdict {
    Yes,
    No,
    Unknown,
    Unsupported,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Yes => "YES",
            Self::No => "NO",
            Self::Unknown => "UNKNOWN",
            Self::Unsupported => "UNSUPPORTED",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Witness {
    pub value: String,
    pub relation: String,
    pub codepoints: usize,
}

impl Witness {
    pub fn new(value: String, relation: impl Into<String>) -> Self {
        let codepoints = value.chars().count();
        Self {
            value,
            relation: relation.into(),
            codepoints,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Diagnostic {
    pub id: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<FrontendError>,
    pub assumptions: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Timings {
    pub parsing_ms: u128,
    pub automata_build_ms: u128,
    pub backend_ms: u128,
    pub witness_extraction_ms: u128,
    pub witness_validation_ms: u128,
    pub rendering_ms: u128,
    pub total_ms: u128,
}

impl Timings {
    pub fn refresh_total(&mut self) {
        self.total_ms = self.parsing_ms
            + self.automata_build_ms
            + self.backend_ms
            + self.witness_extraction_ms
            + self.witness_validation_ms
            + self.rendering_ms;
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Statistics {
    pub left_nfa_states: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right_nfa_states: Option<usize>,
    pub visited_product_states: usize,
    pub generated_transitions: usize,
    pub max_product_states: usize,
    pub timeout_ms: u64,
    pub alphabet: Alphabet,
    pub timings: Timings,
}

#[derive(Clone, Debug, Serialize)]
pub struct Semantics {
    pub match_mode: &'static str,
    pub witness_order: &'static str,
    pub shorthand_classes: &'static str,
    pub dot_matches_newline: bool,
    pub bounded: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct BackendInfo {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub schema_version: &'static str,
    pub tool_version: &'static str,
    pub backend: BackendInfo,
    pub query: Query,
    pub verdict: Verdict,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub witness: Option<Witness>,
    pub diagnostic: Diagnostic,
    pub statistics: Statistics,
    pub semantics: Semantics,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn total_includes_every_reported_phase() {
        let mut timings = Timings {
            parsing_ms: 1,
            automata_build_ms: 2,
            backend_ms: 3,
            witness_extraction_ms: 4,
            witness_validation_ms: 5,
            rendering_ms: 6,
            total_ms: 0,
        };
        timings.refresh_total();
        assert_eq!(timings.total_ms, 21);
    }
}
