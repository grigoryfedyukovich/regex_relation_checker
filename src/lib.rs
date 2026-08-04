#![forbid(unsafe_code)]

//! Library API for `regexrel`.
//!
//! The implementation deliberately targets a documented regular subset and
//! uses full-string semantics. It never treats a bounded search as a proof.

pub mod analysis;
pub mod ast;
pub mod charset;
pub mod config;
pub mod minimize;
pub mod nfa;
pub mod parser;
pub mod report;

pub use analysis::{
    analyze_binary, analyze_binary_with_backend, analyze_empty, analyze_empty_with_backend,
    AnalyzeError, AutomataBackend, BackendResult, BackendStatus, Query, RelationBackend,
};
pub use config::{Alphabet, Config};
pub use minimize::MinimizedBackend;
pub use parser::{parse, FrontendError, FrontendErrorKind};
pub use report::{BackendInfo, Report, Verdict, Witness};
