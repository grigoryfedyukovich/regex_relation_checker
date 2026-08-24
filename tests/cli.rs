use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temporary_config(contents: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("regexrel-{}-{nonce}.toml", std::process::id()));
    fs::write(&path, contents).unwrap();
    path
}

#[test]
fn overlap_running_example() {
    Command::cargo_bin("regexrel")
        .unwrap()
        .args(["overlap", "a+b", "ab+"])
        .assert()
        .success()
        .stdout(predicate::str::contains("YES\nshortest witness: \"ab\""));
}

#[test]
fn includes_running_example() {
    Command::cargo_bin("regexrel")
        .unwrap()
        .args(["includes", "[a-z]+", "[a-z]{2,}"])
        .assert()
        .success()
        .stdout(predicate::str::contains("NO\nwitness in left only: \"a\""));
}

#[test]
fn equivalent_running_example() {
    Command::cargo_bin("regexrel")
        .unwrap()
        .args(["equivalent", "a|b", "[ab]"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("YES\n"));
}

#[test]
fn empty_reports_the_empty_string_as_a_witness() {
    Command::cargo_bin("regexrel")
        .unwrap()
        .args(["empty", ""])
        .assert()
        .success()
        .stdout("NO\nwitness in language: \"\"\n");
}

#[test]
fn empty_character_class_language_is_reported_as_empty() {
    Command::cargo_bin("regexrel")
        .unwrap()
        .args(["empty", r"[^\d\D]"])
        .assert()
        .success()
        .stdout("YES\n");
}

#[test]
fn disjoint_overlap_has_no_witness() {
    Command::cargo_bin("regexrel")
        .unwrap()
        .args(["overlap", "a+", "b+"])
        .assert()
        .success()
        .stdout("NO\n");
}

#[test]
fn equivalence_reports_right_only_direction() {
    Command::cargo_bin("regexrel")
        .unwrap()
        .args(["equivalent", "b", "a|b"])
        .assert()
        .success()
        .stdout("NO\nwitness in right only: \"a\"\n");
}

#[test]
fn anchors_are_documentary_under_full_string_semantics() {
    Command::cargo_bin("regexrel")
        .unwrap()
        .args(["equivalent", "^ab$", "ab"])
        .assert()
        .success()
        .stdout("YES\n");
}

#[test]
fn unicode_mode_accepts_unicode_literals() {
    Command::cargo_bin("regexrel")
        .unwrap()
        .args(["--alphabet", "unicode", "equivalent", "é+", "éé*"])
        .assert()
        .success()
        .stdout("YES\n");
}

#[test]
fn ascii_mode_reports_unicode_literal_as_unsupported() {
    Command::cargo_bin("regexrel")
        .unwrap()
        .args(["equivalent", "é", "é"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("UNSUPPORTED")
                .and(predicate::str::contains(
                    "outside the configured ASCII alphabet",
                ))
                .and(predicate::str::contains("input: left")),
        );
}

#[test]
fn dot_excludes_newline_by_default() {
    Command::cargo_bin("regexrel")
        .unwrap()
        .args(["overlap", ".", r"\n"])
        .assert()
        .success()
        .stdout("NO\n");
}

#[test]
fn dot_can_include_newline() {
    Command::cargo_bin("regexrel")
        .unwrap()
        .args(["--dot-matches-newline", "true", "overlap", ".", r"\n"])
        .assert()
        .success()
        .stdout("YES\nshortest witness: \"\\n\"\n");
}

#[test]
fn json_is_versioned_and_structured() {
    let output = Command::cargo_bin("regexrel")
        .unwrap()
        .args(["--json", "includes", "[a-z]+", "[a-z]{2,}"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schema_version"].as_str(), Some("1"));
    assert_eq!(report["query"].as_str(), Some("includes"));
    assert_eq!(report["verdict"].as_str(), Some("NO"));
    assert_eq!(report["witness"]["value"].as_str(), Some("a"));
    assert_eq!(report["witness"]["relation"].as_str(), Some("left_only"));
    assert_eq!(
        report["semantics"]["match_mode"].as_str(),
        Some("full_string")
    );
    assert_eq!(report["semantics"]["bounded"].as_bool(), Some(false));
    assert_eq!(
        report["backend"]["name"].as_str(),
        Some("in_process_automata")
    );

    let timings = &report["statistics"]["timings"];
    let component_sum = [
        "parsing_ms",
        "automata_build_ms",
        "backend_ms",
        "witness_extraction_ms",
        "witness_validation_ms",
        "rendering_ms",
    ]
    .into_iter()
    .map(|field| timings[field].as_u64().unwrap())
    .sum::<u64>();
    assert_eq!(timings["total_ms"].as_u64(), Some(component_sum));

    let assumptions = report["diagnostic"]["assumptions"].as_array().unwrap();
    assert!(assumptions
        .iter()
        .any(|value| value.as_str() == Some("max_repeat is 1000")));
    assert!(assumptions
        .iter()
        .any(|value| value.as_str() == Some("dot_matches_newline is false")));
}

#[test]
fn stats_add_automata_and_timing_lines() {
    Command::cargo_bin("regexrel")
        .unwrap()
        .args(["--stats", "overlap", "a+b", "ab+"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("states: left=")
                .and(predicate::str::contains("transitions="))
                .and(predicate::str::contains("timing: parse=")),
        );
}

#[test]
fn syntax_command_documents_the_subset() {
    Command::cargo_bin("regexrel")
        .unwrap()
        .arg("syntax")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Supported regex subset")
                .and(predicate::str::contains("{m,n}"))
                .and(predicate::str::contains("backreferences")),
        );
}

#[test]
fn print_config_works_without_a_query() {
    Command::cargo_bin("regexrel")
        .unwrap()
        .arg("--print-config")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("alphabet = \"ascii\"")
                .and(predicate::str::contains("max_product_states = 100000"))
                .and(predicate::str::contains("timeout_ms = 5000")),
        );
}

#[test]
fn cli_flags_override_the_config_file() {
    let path = temporary_config(
        r#"
alphabet = "ascii"
max_product_states = 100000
timeout_ms = 5000
max_repeat = 1000
dot_matches_newline = false
ci_exit_code = 10
"#,
    );

    Command::cargo_bin("regexrel")
        .unwrap()
        .args([
            "--config",
            path.to_str().unwrap(),
            "--alphabet",
            "unicode",
            "equivalent",
            "é",
            "é",
        ])
        .assert()
        .success()
        .stdout("YES\n");

    fs::remove_file(path).unwrap();
}

#[test]
fn cli_override_can_repair_an_invalid_config_value() {
    let path = temporary_config("ci_exit_code = 0\n");

    Command::cargo_bin("regexrel")
        .unwrap()
        .args([
            "--config",
            path.to_str().unwrap(),
            "--ci-exit-code",
            "19",
            "--print-config",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("ci_exit_code = 19"));

    fs::remove_file(path).unwrap();
}

#[test]
fn unknown_config_keys_are_errors() {
    let path = temporary_config("mystery_option = true\n");

    Command::cargo_bin("regexrel")
        .unwrap()
        .args(["--config", path.to_str().unwrap(), "--print-config"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unknown field `mystery_option`"));

    fs::remove_file(path).unwrap();
}

#[test]
fn invalid_zero_state_limit_is_a_configuration_error() {
    Command::cargo_bin("regexrel")
        .unwrap()
        .args(["--max-states", "0", "equivalent", "a", "a"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "max_product_states must be greater than zero",
        ));
}

#[test]
fn ci_policy_uses_configured_exit_code() {
    Command::cargo_bin("regexrel")
        .unwrap()
        .args([
            "--fail-on",
            "no",
            "--ci-exit-code",
            "17",
            "equivalent",
            "a",
            "b",
        ])
        .assert()
        .code(17);
}

#[test]
fn fail_on_unknown_also_catches_unsupported() {
    Command::cargo_bin("regexrel")
        .unwrap()
        .args([
            "--fail-on",
            "unknown",
            "--ci-exit-code",
            "23",
            "equivalent",
            r"(a)\1",
            "a",
        ])
        .assert()
        .code(23)
        .stdout(predicate::str::starts_with("UNSUPPORTED\n"));
}

#[test]
fn state_limit_is_unknown_and_can_fail_ci() {
    Command::cargo_bin("regexrel")
        .unwrap()
        .args([
            "--max-states",
            "1",
            "--fail-on",
            "unknown",
            "--ci-exit-code",
            "29",
            "equivalent",
            "a",
            "b",
        ])
        .assert()
        .code(29)
        .stdout(
            predicate::str::contains("UNKNOWN")
                .and(predicate::str::contains("product-state limit"))
                .and(predicate::str::contains("product=1 / 1")),
        );
}

#[test]
fn repetition_overflow_has_a_specific_error() {
    let pattern = format!("a{{{}}}", "9".repeat(100));
    Command::cargo_bin("regexrel")
        .unwrap()
        .arg("empty")
        .arg(&pattern)
        .assert()
        .code(2)
        .stderr(
            predicate::str::contains("repetition bound overflows the representable range")
                .and(predicate::str::contains("use a smaller bound")),
        );
}

#[test]
fn syntax_error_is_distinct() {
    Command::cargo_bin("regexrel")
        .unwrap()
        .args(["overlap", "(", "a"])
        .assert()
        .code(2)
        .stderr(
            predicate::str::contains("input error [syntax]")
                .and(predicate::str::contains("unterminated group")),
        );
}

#[test]
fn draw_nfa_emits_dot_for_a_single_regex() {
    Command::cargo_bin("regexrel")
        .unwrap()
        .args(["--draw", "nfa", "--emit-dot", "a+b"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("digraph")
                .and(predicate::str::contains("label=\"a\""))
                .and(predicate::str::contains("label=\"b\""))
                .and(predicate::str::contains("label=\"ε\"")),
        );
}

#[test]
fn draw_subcommand_matches_the_flag() {
    Command::cargo_bin("regexrel")
        .unwrap()
        .args(["draw", "dfa", "--emit-dot", "a|b"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("digraph")
                .and(predicate::str::contains("label=\"a\""))
                .and(predicate::str::contains("Minimized DFA").not())
                .and(predicate::str::contains("DFA of a|b")),
        );
}

#[test]
fn draw_minimized_emits_dot() {
    Command::cargo_bin("regexrel")
        .unwrap()
        .args(["--draw=minimized", "--emit-dot", "[ab]"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("digraph")
                .and(predicate::str::contains("Minimized DFA of [ab]")),
        );
}

#[test]
fn draw_writes_a_dot_file_without_graphviz() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("regexrel-draw-{nonce}.dot"));
    Command::cargo_bin("regexrel")
        .unwrap()
        .args(["--draw", "nfa", "--output", path.to_str().unwrap(), "ab"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("wrote")
                .and(predicate::str::contains("states"))
                .and(predicate::str::contains("transitions")),
        );
    let contents = fs::read_to_string(&path).unwrap();
    fs::remove_file(&path).ok();
    assert!(contents.contains("digraph"));
    assert!(contents.contains("label=\"a\""));
    assert!(contents.contains("label=\"b\""));
}

#[test]
fn draw_rejects_an_unknown_kind() {
    Command::cargo_bin("regexrel")
        .unwrap()
        .args(["--draw", "pda", "a"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("invalid value 'pda'")
                .or(predicate::str::contains("invalid value")),
        );
}

#[test]
fn draw_requires_a_regex() {
    Command::cargo_bin("regexrel")
        .unwrap()
        .args(["--draw", "nfa"])
        .assert()
        .failure();
}

#[test]
fn draw_reports_syntax_errors() {
    Command::cargo_bin("regexrel")
        .unwrap()
        .args(["--draw", "nfa", "--emit-dot", "("])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("input error [syntax]"));
}
