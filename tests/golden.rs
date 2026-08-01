use assert_cmd::Command;
use std::fs;
use std::path::Path;

fn assert_golden(name: &str, args: &[&str]) {
    let expected = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("golden")
            .join(format!("{name}.stdout")),
    )
    .unwrap();

    Command::cargo_bin("regexrel")
        .unwrap()
        .args(args)
        .assert()
        .success()
        .stdout(expected);
}

#[test]
fn stable_text_reports_match_golden_files() {
    let cases: &[(&str, &[&str])] = &[
        ("overlap_yes", &["overlap", "a+b", "ab+"]),
        ("overlap_no", &["overlap", "a+", "b+"]),
        ("includes_no", &["includes", "[a-z]+", "[a-z]{2,}"]),
        ("equivalent_yes", &["equivalent", "a|b", "[ab]"]),
        ("equivalent_right_only", &["equivalent", "b", "a|b"]),
        ("empty_string_witness", &["empty", ""]),
    ];

    for (name, args) in cases {
        assert_golden(name, args);
    }
}
