#![forbid(unsafe_code)]

use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use regexrel::analysis::AnalyzeError;
use regexrel::config::{Alphabet, Config};
use regexrel::parser::SYNTAX_HELP;
use regexrel::report::{relation, Report, Timings, Verdict};
use regexrel::{
    analyze_binary_with_backend, analyze_empty_with_backend, AutomataBackend, MinimizedBackend,
    Query, RelationBackend,
};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const INPUT_ERROR_EXIT: u8 = 2;
const INTERNAL_ERROR_EXIT: u8 = 3;
const TIMING_PLACEHOLDER: &str = "__REGEXREL_TIMING_LINE__";

#[derive(Debug, Parser)]
#[command(
    name = "regexrel",
    version,
    about = "Check relations between regular-expression languages"
)]
struct Cli {
    /// TOML configuration file. If omitted, ./regexrel.toml is loaded when present.
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Alphabet used by dot and negated classes.
    #[arg(long, value_enum, global = true)]
    alphabet: Option<AlphabetArg>,

    /// Maximum number of on-the-fly product states.
    #[arg(long = "max-states", global = true)]
    max_product_states: Option<usize>,

    /// Analysis timeout in milliseconds.
    #[arg(long, global = true)]
    timeout_ms: Option<u64>,

    /// Maximum accepted explicit repetition bound.
    #[arg(long, global = true)]
    max_repeat: Option<usize>,

    /// Whether '.' includes newline. Pass true or false.
    #[arg(long, action = ArgAction::Set, global = true)]
    dot_matches_newline: Option<bool>,

    /// Emit the versioned machine-readable report.
    #[arg(long, global = true)]
    json: bool,

    /// Include automata and timing statistics in text output.
    #[arg(long, global = true)]
    stats: bool,

    /// Print the effective configuration before analysis, or by itself.
    #[arg(long, global = true)]
    print_config: bool,

    /// CI policy controlling when an otherwise successful run exits nonzero.
    #[arg(long, value_enum, default_value = "never", global = true)]
    fail_on: FailOn,

    /// Nonzero exit code used when --fail-on triggers.
    #[arg(long, global = true)]
    ci_exit_code: Option<i32>,

    /// Analysis engine to use. Both implement the same regular subset and
    /// agree on every verdict; this exists to let the two be cross-checked
    /// against each other, and as a size/technique tradeoff for advanced use.
    #[arg(long, value_enum, default_value = "automata", global = true)]
    backend: BackendArg,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum BackendArg {
    /// On-the-fly subset construction with a single product-BFS over both
    /// patterns at once. The default; well-exercised, no upfront cost.
    #[default]
    Automata,
    /// Determinize and minimize each pattern first, then check equivalence
    /// via canonical-form isomorphism (no search needed when it holds) or
    /// fall back to a product search over the minimized DFAs otherwise.
    Minimized,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum AlphabetArg {
    Ascii,
    Unicode,
}

impl From<AlphabetArg> for Alphabet {
    fn from(value: AlphabetArg) -> Self {
        match value {
            AlphabetArg::Ascii => Alphabet::Ascii,
            AlphabetArg::Unicode => Alphabet::Unicode,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
enum FailOn {
    #[default]
    Never,
    No,
    Unknown,
    NonYes,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Decide whether a regex language is empty.
    Empty { regex: String },
    /// Decide whether the two regex languages have a common string.
    Overlap { left: String, right: String },
    /// Decide whether every left-language string is in the right language.
    Includes { left: String, right: String },
    /// Decide whether the two regex languages are equal.
    Equivalent { left: String, right: String },
    /// Print the supported regex subset and semantic notes.
    Syntax,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => ExitCode::from(code),
        Err((code, message)) => {
            eprintln!("{message}");
            ExitCode::from(code)
        }
    }
}

fn run(cli: Cli) -> Result<u8, (u8, String)> {
    let mut config = load_config(cli.config.as_deref())
        .map_err(|error| (INPUT_ERROR_EXIT, format!("configuration error: {error}")))?;
    apply_overrides(&cli, &mut config);
    config = Config::validate(config)
        .map_err(|error| (INPUT_ERROR_EXIT, format!("configuration error: {error}")))?;

    if cli.print_config {
        let rendered = toml::to_string_pretty(&config).map_err(|error| {
            (
                INTERNAL_ERROR_EXIT,
                format!("internal error while rendering configuration: {error}"),
            )
        })?;
        println!("{rendered}");
    }

    let Some(command) = cli.command else {
        if cli.print_config {
            return Ok(0);
        }
        return Err((
            INPUT_ERROR_EXIT,
            "a subcommand is required; run 'regexrel --help'".to_owned(),
        ));
    };

    if matches!(&command, Command::Syntax) {
        print!("{SYNTAX_HELP}");
        return Ok(0);
    }

    let backend: &dyn RelationBackend = match cli.backend {
        BackendArg::Automata => &AutomataBackend,
        BackendArg::Minimized => &MinimizedBackend,
    };

    let mut report = match command {
        Command::Empty { regex } => analyze_empty_with_backend(&regex, &config, backend),
        Command::Overlap { left, right } => {
            analyze_binary_with_backend(Query::Overlap, &left, &right, &config, backend)
        }
        Command::Includes { left, right } => {
            analyze_binary_with_backend(Query::Includes, &left, &right, &config, backend)
        }
        Command::Equivalent { left, right } => {
            analyze_binary_with_backend(Query::Equivalent, &left, &right, &config, backend)
        }
        Command::Syntax => unreachable!(),
    }
    .map_err(render_analysis_error)?;

    if cli.json {
        let previous_rendering_ms = report.statistics.timings.rendering_ms;
        let previous_total_ms = report.statistics.timings.total_ms;
        let rendering_started = std::time::Instant::now();
        let mut json = serde_json::to_string_pretty(&report).map_err(|error| {
            (
                INTERNAL_ERROR_EXIT,
                format!("internal error while rendering JSON: {error}"),
            )
        })?;
        report.statistics.timings.rendering_ms = rendering_started.elapsed().as_millis();
        report.statistics.timings.refresh_total();
        patch_json_number(
            &mut json,
            "rendering_ms",
            previous_rendering_ms,
            report.statistics.timings.rendering_ms,
        )?;
        patch_json_number(
            &mut json,
            "total_ms",
            previous_total_ms,
            report.statistics.timings.total_ms,
        )?;
        println!("{json}");
    } else {
        let rendering_started = std::time::Instant::now();
        let mut text = render_text(&report, cli.stats);
        report.statistics.timings.rendering_ms = rendering_started.elapsed().as_millis();
        report.statistics.timings.refresh_total();
        if let Some(start) = text.rfind(TIMING_PLACEHOLDER) {
            let timing_line = render_timing_line(&report.statistics.timings);
            text.replace_range(start..start + TIMING_PLACEHOLDER.len(), &timing_line);
        }
        print!("{text}");
    }

    if policy_fails(cli.fail_on, report.verdict) {
        Ok(config.ci_exit_code as u8)
    } else {
        Ok(0)
    }
}

fn load_config(explicit: Option<&Path>) -> Result<Config, regexrel::config::ConfigError> {
    if let Some(path) = explicit {
        return Config::load_raw(path);
    }
    let local = Path::new("regexrel.toml");
    if local.is_file() {
        Config::load_raw(local)
    } else {
        Ok(Config::default())
    }
}

fn apply_overrides(cli: &Cli, config: &mut Config) {
    if let Some(alphabet) = cli.alphabet {
        config.alphabet = alphabet.into();
    }
    if let Some(limit) = cli.max_product_states {
        config.max_product_states = limit;
    }
    if let Some(timeout) = cli.timeout_ms {
        config.timeout_ms = timeout;
    }
    if let Some(limit) = cli.max_repeat {
        config.max_repeat = limit;
    }
    if let Some(value) = cli.dot_matches_newline {
        config.dot_matches_newline = value;
    }
    if let Some(code) = cli.ci_exit_code {
        config.ci_exit_code = code;
    }
}

fn patch_json_number(
    json: &mut String,
    field: &str,
    previous: u128,
    current: u128,
) -> Result<(), (u8, String)> {
    let needle = format!("\"{field}\": {previous}");
    let replacement = format!("\"{field}\": {current}");
    let Some(start) = json.find(&needle) else {
        return Err((
            INTERNAL_ERROR_EXIT,
            format!("internal error while finalizing JSON timing field {field:?}"),
        ));
    };
    json.replace_range(start..start + needle.len(), &replacement);
    Ok(())
}

fn render_timing_line(timings: &Timings) -> String {
    format!(
        "timing: parse={} ms, build={} ms, backend={} ms, extract={} ms, validate={} ms, render={} ms, total={} ms",
        timings.parsing_ms,
        timings.automata_build_ms,
        timings.backend_ms,
        timings.witness_extraction_ms,
        timings.witness_validation_ms,
        timings.rendering_ms,
        timings.total_ms,
    )
}

fn render_analysis_error(error: AnalyzeError) -> (u8, String) {
    match error {
        AnalyzeError::Input(error) => {
            let mut message = format!(
                "input error [{}] at bytes {}..{}: {}",
                match error.kind {
                    regexrel::FrontendErrorKind::Syntax => "syntax",
                    regexrel::FrontendErrorKind::Unsupported => "unsupported",
                },
                error.span_start,
                error.span_end,
                error.message
            );
            if let Some(hint) = error.hint {
                message.push_str(&format!("\nhint: {hint}"));
            }
            (INPUT_ERROR_EXIT, message)
        }
        AnalyzeError::Internal(message) => {
            let command = std::env::args()
                .map(|arg| format!("{arg:?}"))
                .collect::<Vec<_>>()
                .join(" ");
            (
                INTERNAL_ERROR_EXIT,
                format!(
                    "internal error: {message}\nreproduce locally with: {command}\nno data was uploaded"
                ),
            )
        }
    }
}

fn render_text(report: &Report, show_stats: bool) -> String {
    let mut lines = vec![report.verdict.as_str().to_owned()];
    if let Some(witness) = &report.witness {
        let escaped =
            serde_json::to_string(&witness.value).unwrap_or_else(|_| "<invalid>".to_owned());
        let line = match report.query {
            Query::Overlap => format!("shortest witness: {escaped}"),
            Query::Includes => format!("witness in left only: {escaped}"),
            Query::Equivalent => match witness.relation.as_str() {
                relation::LEFT_ONLY => format!("witness in left only: {escaped}"),
                relation::RIGHT_ONLY => format!("witness in right only: {escaped}"),
                other => format!("distinguishing witness ({other}): {escaped}"),
            },
            Query::Empty => format!("witness in language: {escaped}"),
        };
        lines.push(line);
    }
    if matches!(report.verdict, Verdict::Unknown | Verdict::Unsupported) {
        lines.push(format!("reason: {}", report.diagnostic.message));
        if let Some(input) = &report.diagnostic.input {
            lines.push(format!("input: {input}"));
        }
        if let Some(error) = &report.diagnostic.error {
            lines.push(format!(
                "location: bytes {}..{}",
                error.span_start, error.span_end
            ));
            if let Some(hint) = &error.hint {
                lines.push(format!("hint: {hint}"));
            }
        }
    }
    if show_stats || report.verdict == Verdict::Unknown {
        lines.push(format!(
            "states: left={}, right={}, product={} / {}; transitions={}; timeout={} ms",
            report.statistics.left_nfa_states,
            report
                .statistics
                .right_nfa_states
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            report.statistics.visited_product_states,
            report.statistics.max_product_states,
            report.statistics.generated_transitions,
            report.statistics.timeout_ms,
        ));
        lines.push(TIMING_PLACEHOLDER.to_owned());
    }
    lines.push(String::new());
    lines.join("\n")
}

fn policy_fails(policy: FailOn, verdict: Verdict) -> bool {
    match policy {
        FailOn::Never => false,
        FailOn::No => verdict == Verdict::No,
        FailOn::Unknown => matches!(verdict, Verdict::Unknown | Verdict::Unsupported),
        FailOn::NonYes => verdict != Verdict::Yes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_timing_fields_are_patched_without_rerendering() {
        let mut json = r#"{"rendering_ms": 0, "total_ms": 7}"#.to_owned();
        patch_json_number(&mut json, "rendering_ms", 0, 3).unwrap();
        patch_json_number(&mut json, "total_ms", 7, 10).unwrap();
        assert_eq!(json, r#"{"rendering_ms": 3, "total_ms": 10}"#);
    }

    #[test]
    fn missing_json_timing_field_is_an_internal_error() {
        let mut json = "{}".to_owned();
        let error = patch_json_number(&mut json, "total_ms", 0, 1).unwrap_err();
        assert_eq!(error.0, INTERNAL_ERROR_EXIT);
    }
}
