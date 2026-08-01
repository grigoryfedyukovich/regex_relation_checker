use regexrel::{analyze_binary, Config, Query, Verdict};

#[test]
fn route_and_identifier_policy_corpus() {
    let cases = [
        (
            Query::Includes,
            "[a-z0-9-]+",
            "[a-z][a-z0-9-]*",
            Verdict::No,
            Some("-"),
        ),
        (Query::Equivalent, r"v[0-9]+", r"v\d+", Verdict::Yes, None),
        (Query::Overlap, r"admin/.*", r"public/.*", Verdict::No, None),
        (
            Query::Equivalent,
            r"item(s)?",
            r"items?",
            Verdict::Yes,
            None,
        ),
        (
            Query::Includes,
            "[0-9]+",
            "[1-9][0-9]*",
            Verdict::No,
            Some("0"),
        ),
        (Query::Overlap, "[a-c]+", "[c-e]+", Verdict::Yes, Some("c")),
        (Query::Overlap, "a*", "b*", Verdict::Yes, Some("")),
        (Query::Includes, r"\d+", r"\w+", Verdict::Yes, None),
        (Query::Overlap, r"\w+", r"\d+", Verdict::Yes, Some("0")),
        (Query::Equivalent, r"\+", "[+]", Verdict::Yes, None),
        (Query::Equivalent, "a{2,3}", "aa(a?)", Verdict::Yes, None),
        (Query::Includes, "a*", "a+", Verdict::No, Some("")),
        (Query::Includes, "[ab]", "a|b", Verdict::Yes, None),
        (
            Query::Equivalent,
            "^api/[a-z]+$",
            "api/[a-z]+",
            Verdict::Yes,
            None,
        ),
        (Query::Overlap, "[^a]", "a", Verdict::No, None),
        (Query::Equivalent, "b", "a|b", Verdict::No, Some("a")),
    ];

    for (query, left, right, verdict, witness) in cases {
        let report = analyze_binary(query, left, right, &Config::default()).unwrap();
        assert_eq!(report.verdict, verdict, "{query:?}: {left:?}, {right:?}");
        assert_eq!(
            report.witness.as_ref().map(|value| value.value.as_str()),
            witness,
            "{query:?}: {left:?}, {right:?}",
        );
    }
}
