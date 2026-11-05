mod common;

use common::{c, v};
use semver_resolver::Constraint;

#[test]
fn compiles_to_the_expected_intervals() {
    let table = [
        ("^1.2.3", ">=1.2.3 <2.0.0-0"),
        ("^0.2.3", ">=0.2.3 <0.3.0-0"),
        ("^0.0.3", ">=0.0.3 <0.0.4-0"),
        ("^1.2", ">=1.2.0 <2.0.0-0"),
        ("^1.2.x", ">=1.2.0 <2.0.0-0"),
        ("^1", ">=1.0.0 <2.0.0-0"),
        ("^1.x", ">=1.0.0 <2.0.0-0"),
        ("^0.2", ">=0.2.0 <0.3.0-0"),
        ("^0.0", ">=0.0.0 <0.1.0-0"),
        ("^0.0.x", ">=0.0.0 <0.1.0-0"),
        ("^0", ">=0.0.0 <1.0.0-0"),
        ("^*", "*"),
        ("~1.2.3", ">=1.2.3 <1.3.0-0"),
        ("~0.2.3", ">=0.2.3 <0.3.0-0"),
        ("~1.2", ">=1.2.0 <1.3.0-0"),
        ("~1", ">=1.0.0 <2.0.0-0"),
        ("~>1.2", ">=1.2.0 <1.3.0-0"),
        ("~*", "*"),
        ("1.2.3", "=1.2.3"),
        ("=1.2.3", "=1.2.3"),
        ("v1.2.3", "=1.2.3"),
        ("=v1.2.3", "=1.2.3"),
        ("1.2.3+meta", "=1.2.3"),
        ("1.2", ">=1.2.0 <1.3.0-0"),
        ("=1.2", ">=1.2.0 <1.3.0-0"),
        ("1.2.x", ">=1.2.0 <1.3.0-0"),
        ("1.2.*", ">=1.2.0 <1.3.0-0"),
        ("1.X", ">=1.0.0 <2.0.0-0"),
        ("1.*.*", ">=1.0.0 <2.0.0-0"),
        ("1", ">=1.0.0 <2.0.0-0"),
        ("*", "*"),
        ("x", "*"),
        ("*.*.*", "*"),
        ("", "*"),
        ("   ", "*"),
        (">1.2.3", ">1.2.3"),
        (">1.2", ">=1.3.0"),
        (">1", ">=2.0.0"),
        (">=1.2.3", ">=1.2.3"),
        (">=1.2", ">=1.2.0"),
        ("<1.2.3", "<1.2.3"),
        ("<1.2", "<1.2.0-0"),
        ("<1", "<1.0.0-0"),
        ("<=1.2.3", "<=1.2.3"),
        ("<=1.2", "<1.3.0-0"),
        ("<=1", "<2.0.0-0"),
        (">=*", "*"),
        ("<=*", "*"),
        (">*", "<none>"),
        ("<*", "<none>"),
        ("<0", "<none>"),
        ("1.2.3 - 2.3.4", ">=1.2.3 <=2.3.4"),
        ("1.2 - 2.3.4", ">=1.2.0 <=2.3.4"),
        ("1.2.3 - 2.3", ">=1.2.3 <2.4.0-0"),
        ("1.2.3 - 2", ">=1.2.3 <3.0.0-0"),
        ("* - 2", "<3.0.0-0"),
    ];
    for (input, expected) in table {
        assert_eq!(c(input).to_string(), expected, "for input {input:?}");
    }
}

#[test]
fn and_separators_are_interchangeable() {
    for input in [">=1.0.0 <2.0.0", ">=1.0.0,<2.0.0", ">= 1.0.0 , < 2.0.0", ">=1.0.0<2.0.0"] {
        assert_eq!(c(input).to_string(), ">=1.0.0 <2.0.0", "for input {input:?}");
    }
}

#[test]
fn alternatives_sort_and_merge() {
    let table = [
        ("^2 || ^1", ">=1.0.0 <2.0.0-0 || >=2.0.0 <3.0.0-0"),
        (">=1 <2 || >=1.5 <3", ">=1.0.0 <3.0.0-0"),
        ("<1.0.0 || >=1.0.0", "*"),
        ("<1.0.0 || >1.0.0", "<1.0.0 || >1.0.0"),
        ("<=1.0.0 || >1.0.0", "*"),
        (">=1 || >=2", ">=1.0.0"),
        ("1.2.3 || 1.2.3", "=1.2.3"),
        ("* || ^1", "*"),
        (">2 <1 || 3.0.0", "=3.0.0"),
        (">2 <1 || >4 <3", "<none>"),
    ];
    for (input, expected) in table {
        assert_eq!(c(input).to_string(), expected, "for input {input:?}");
    }
}

#[test]
fn inverted_and_touching_bounds_are_empty_or_exact() {
    assert!(c(">2 <1").is_empty());
    assert!(c(">1.0.0 <1.0.0").is_empty());
    assert!(c(">=1.0.0 <1.0.0").is_empty());
    assert!(c(">1.0.0 <=1.0.0").is_empty());
    assert_eq!(c(">=1.0.0 <=1.0.0").to_string(), "=1.0.0");
    assert!(!c(">2 <1").matches(&v("1.5.0")));
}

#[test]
fn equality_is_by_meaning_and_source_is_kept() {
    let spelled = c("^1.2.3");
    assert_eq!(spelled, c(">=1.2.3, <2.0.0-0"));
    assert_ne!(spelled, c("^1.2.4"));
    assert_eq!(spelled.source(), "^1.2.3");
    assert_eq!(Constraint::any(), c("*"));
    assert_eq!(Constraint::exact(&v("1.2.3+meta")), c("=1.2.3"));
}

#[test]
fn matches_release_versions_by_bounds() {
    let cases = [
        ("^1.2.3", "1.2.3", true),
        ("^1.2.3", "1.9.9", true),
        ("^1.2.3", "1.2.2", false),
        ("^1.2.3", "2.0.0", false),
        ("^0.2.3", "0.3.0", false),
        ("^0.0.3", "0.0.4", false),
        ("~1.2.3", "1.2.9", true),
        ("~1.2.3", "1.3.0", false),
        ("1.2.3 - 2.0.0", "2.0.0", true),
        ("1.2.3 - 2.0.0", "2.0.1", false),
        (">1.2.3", "1.2.3", false),
        ("<1.2.3", "1.2.3", false),
        ("1.x || >=3.1", "2.5.0", false),
        ("1.x || >=3.1", "3.1.0", true),
        ("=1.2.3", "1.2.3+build", true),
        ("=1.2.3+x", "1.2.3+y", true),
    ];
    for (constraint, version, expected) in cases {
        assert_eq!(c(constraint).matches(&v(version)), expected, "{constraint} on {version}");
    }
}

#[test]
fn prereleases_need_a_comparator_on_the_same_core() {
    let cases = [
        ("^1.2.3-beta.2", "1.2.3-beta.4", true),
        ("^1.2.3-beta.2", "1.2.3-beta.1", false),
        ("^1.2.3-beta.2", "1.2.3", true),
        ("^1.2.3-beta.2", "1.3.0", true),
        ("^1.2.3-beta.2", "1.2.4-beta.2", false),
        ("^1.2.3-beta.2", "2.0.0-alpha", false),
        (">1.2.3-alpha.3", "1.2.3-alpha.7", true),
        (">1.2.3-alpha.3", "3.4.5-alpha.9", false),
        (">1.2.3-alpha.3", "3.4.5", true),
        ("~1.2.3-rc.1", "1.2.3-rc.2", true),
        ("~1.2.3-rc.1", "1.2.9-rc.1", false),
        ("=1.2.3-rc.1", "1.2.3-rc.1", true),
        ("1.2.3-rc.1", "1.2.3-rc.1+build", true),
        ("<2.0.0-beta", "2.0.0-alpha", true),
        ("<2.0.0", "2.0.0-alpha", false),
        ("<2.0.0", "1.5.0-rc.1", false),
        ("*", "1.0.0-rc.1", false),
        (">=1.0.0", "2.0.0-alpha", false),
        (">=1.0.0-alpha <=2.0.0-beta", "1.0.0-beta", true),
        (">=1.0.0-alpha <=2.0.0-beta", "2.0.0-alpha", true),
        (">=1.0.0-alpha <=2.0.0-beta", "1.5.0-rc.1", false),
        ("1.0.0-alpha - 1.0.0-rc", "1.0.0-beta", true),
    ];
    for (constraint, version, expected) in cases {
        assert_eq!(c(constraint).matches(&v(version)), expected, "{constraint} on {version}");
    }
}

#[test]
fn prerelease_permission_stays_within_its_alternative() {
    let constraint = c(">=1.0.0-alpha <1.0.1 || >=2.0.0");
    assert!(constraint.matches(&v("1.0.0-beta")));
    assert!(constraint.matches(&v("2.3.0")));
    assert!(!constraint.matches(&v("2.3.0-beta")));
}

#[test]
fn opting_in_opens_bounds_but_ceilings_still_hold() {
    let cases = [
        ("*", "1.0.0-rc.1", true),
        ("^1.0.0", "1.5.0-beta", true),
        ("^1.0.0", "1.0.0-alpha", false),
        ("^1.0.0", "2.0.0-alpha", false),
        ("~1.2.0", "1.3.0-alpha", false),
        ("1.x", "2.0.0-alpha", false),
        ("1.x", "1.0.0-alpha", false),
        ("<2.0.0", "2.0.0-alpha", true),
        ("<=1.2", "1.2.5-rc", true),
        ("<=1.2", "1.3.0-rc", false),
        (">=2", "2.0.0-rc", false),
    ];
    for (constraint, version, expected) in cases {
        assert_eq!(
            c(constraint).matches_with(&v(version), true),
            expected,
            "{constraint} on {version}"
        );
    }
}

#[test]
fn intersection_displays_bounds() {
    let table = [
        ("^1.2", "<1.5", ">=1.2.0 <1.5.0-0"),
        ("^1", "^2", "<none>"),
        ("*", "^1", ">=1.0.0 <2.0.0-0"),
        ("^1 || ^3", ">=1.5 <3.5", ">=1.5.0 <2.0.0-0 || >=3.0.0 <3.5.0-0"),
        (">=1.0.0", "<=1.0.0", "=1.0.0"),
        ("<1.0.0 || >2.0.0", ">=1.0.0 <=2.0.0", "<none>"),
    ];
    for (a, b, expected) in table {
        assert_eq!(c(a).intersect(&c(b)).to_string(), expected, "{a} and {b}");
    }
}

#[test]
fn intersection_marks_prerelease_permission_it_cannot_imply() {
    let combined = c(">=1.0.0-alpha").intersect(&c("<=2.0.0-beta"));
    assert_eq!(combined.to_string(), ">=1.0.0-alpha <=2.0.0-beta [pre: none]");
    assert!(!combined.matches(&v("1.0.0-beta")));
    assert!(!combined.matches(&v("2.0.0-alpha")));
    assert!(combined.matches(&v("1.5.0")));

    let shared = c(">=1.0.0-alpha <1.1.0").intersect(&c(">=1.0.0-beta"));
    // Both sides name core 1.0.0, and the bounds already imply that, so no marker.
    assert_eq!(shared.to_string(), ">=1.0.0-beta <1.1.0");
    assert!(shared.matches(&v("1.0.0-beta")));
    assert!(!shared.matches(&v("1.0.0-alpha")));
    assert!(shared.matches(&v("1.0.0")));
    assert_eq!(Constraint::parse(&shared.to_string()).unwrap(), shared);
}

#[test]
fn intersection_agrees_with_testing_both_sides() {
    let constraints = [
        "*",
        "^1.2.3",
        "^1.2.3-beta.1",
        "~1.2",
        "1.x || 3.x",
        ">=1.0.0-alpha <=2.0.0-beta",
        "<2.0.0",
        "<2.0.0-rc",
        ">1.2.3",
        "1.2.3-rc.1",
        "=1.2.3",
        ">=1.0.0-alpha <1.0.1 || >=2.0.0",
        "<1 || >=3.0.0-rc",
        ">5 <1",
    ];
    let versions = [
        "0.9.0",
        "1.0.0-alpha",
        "1.0.0-beta",
        "1.0.0",
        "1.2.2",
        "1.2.3-beta.0",
        "1.2.3-beta.1",
        "1.2.3-rc.1",
        "1.2.3",
        "1.2.9",
        "1.3.0-rc",
        "1.9.0",
        "2.0.0-alpha",
        "2.0.0-beta",
        "2.0.0",
        "2.5.0",
        "3.0.0-rc",
        "3.0.0",
    ];
    for a in constraints {
        for b in constraints {
            let (left, right) = (c(a), c(b));
            let both = left.intersect(&right);
            assert_eq!(both, right.intersect(&left), "{a} and {b} should commute");
            for text in versions {
                let version = v(text);
                for opt_in in [false, true] {
                    assert_eq!(
                        both.matches_with(&version, opt_in),
                        left.matches_with(&version, opt_in)
                            && right.matches_with(&version, opt_in),
                        "{a} and {b} on {text}, opt_in={opt_in}, combined {both}"
                    );
                }
            }
        }
    }
}

#[test]
fn normalized_text_parses_back_to_the_same_constraint() {
    for input in [
        "^1.2.3",
        "^1.2.3-beta.1",
        "~0.0.3",
        "1.x || 3.x || >=9",
        ">=1.0.0-alpha <=2.0.0-beta",
        "<2.0.0-rc",
        "1.2.3 - 2",
        "1.0.0-a - 1.0.0-z",
        ">=1.0.0-alpha >=1.5.0 <2",
        "=1.2.3",
        "*",
    ] {
        let first = c(input);
        let again = Constraint::parse(&first.to_string()).unwrap();
        assert_eq!(first, again, "{input} normalized to {first}");
    }
}

#[test]
fn rejects_bad_syntax_with_a_position() {
    let table = [
        ("^", 0),
        (">=", 0),
        (">= ,1", 0),
        ("abc", 0),
        ("1.2.3.4", 0),
        ("1.x.3", 0),
        ("01.2.3", 0),
        ("1.2-beta", 0),
        ("1.2+meta", 0),
        ("1.2.3-", 0),
        ("1.2.3-01", 0),
        ("1.2.3+", 0),
        ("^1 ||", 5),
        ("|| ^1", 0),
        ("^1 || abc", 6),
        ("^1 || ^2 || ~", 12),
        (">=1 abc", 4),
        ("1.0 -", 4),
        ("- 1.0", 0),
        ("1 - 2 - 3", 2),
        ("1 - 2 <3", 2),
        (">=1 - 2", 4),
        ("1 - >=2", 2),
        ("^18446744073709551615", 0),
        ("^0.18446744073709551615", 0),
        ("~0.0.1.1", 0),
    ];
    for (input, offset) in table {
        let err = Constraint::parse(input).expect_err(input);
        assert_eq!(err.offset, offset, "for input {input:?}: {err}");
        assert_eq!(err.input, input);
    }
}

#[test]
fn error_message_reads_with_column_and_reason() {
    let message = Constraint::parse("^1 || 1.x.3").unwrap_err().to_string();
    assert!(message.contains("column 7"), "{message}");
    assert!(message.contains("number after a wildcard"), "{message}");
}
