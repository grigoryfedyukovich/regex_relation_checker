#![forbid(unsafe_code)]

//! Library API for `regexrel`.
//!
//! The implementation deliberately targets a documented regular subset and
//! uses full-string semantics. It never treats a bounded search as a proof.

pub mod abstraction;
pub mod analysis;
pub mod antichain;
pub mod antimirov;
pub mod ast;
pub mod charset;
pub mod config;
pub mod derivative;
pub mod draw;
pub mod hkc;
pub mod minimize;
pub mod nfa;
pub mod parser;
pub mod report;
mod residual;

pub use abstraction::AbstractionBackend;
pub use analysis::{
    analyze_binary, analyze_binary_with_backend, analyze_empty, analyze_empty_with_backend,
    analyze_match, analyze_match_with_backend, AnalyzeError, AutomataBackend, BackendResult,
    BackendStatus, Orientation, OrientedBackend, Query, RelationBackend, ReversalBackend,
};
pub use antichain::AntichainBackend;
pub use antimirov::AntimirovBackend;
pub use config::{Alphabet, Config};
pub use derivative::DerivativeBackend;
pub use draw::{draw_dot, render_graph, DrawError, DrawKind, DrawResult};
pub use hkc::HkcBackend;
pub use minimize::MinimizedBackend;
pub use parser::{parse, FrontendError, FrontendErrorKind};
pub use report::{
    BackendInfo, OrientationProbeStatistics, OrientationSelection, Report, Verdict, Witness,
};
