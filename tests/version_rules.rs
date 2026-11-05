use semver_resolver::version::Identifier;
use semver_resolver::{Bump, Version};

fn v(text: &str) -> Version {
    Version::parse(text).unwrap()
}

#[test]
fn parses_every_part() {
    let parsed = v("1.2.3-alpha.7.x-y+build.007");
    assert_eq!(parsed.core(), (1, 2, 3));
    assert_eq!(
        parsed.pre,
        vec![
            Identifier::Alpha("alpha".into()),
            Identifier::Numeric(7),
            Identifier::Alpha("x-y".into())
        ]
    );
    assert_eq!(parsed.build, vec!["build".to_string(), "007".to_string()]);
    assert_eq!(parsed.to_string(), "1.2.3-alpha.7.x-y+build.007");
}

#[test]
fn accepts_edge_forms_from_the_spec() {
    for text in [
        "0.0.0",
        "1.0.0--",
        "1.0.0-0a",
        "1.0.0-a-b-c",
        "1.0.0--1",
        "1.0.0+-",
        "1.0.0+001",
        "1.0.0-0.0.0",
        "18446744073709551615.0.0",
    ] {
        assert!(Version::parse(text).is_ok(), "should accept {text}");
        assert_eq!(v(text).to_string(), text);
    }
}

#[test]
fn rejects_malformed_versions() {
    for text in [
        "",
        "1",
        "1.2",
        "1.2.3.4",
        "v1.2.3",
        " 1.2.3",
        "1.2.3 ",
        "01.2.3",
        "1.02.3",
        "1.2.03",
        "1.2.x",
        "-1.2.3",
        "1.2.3-",
        "1.2.3+",
        "1.2.3-01",
        "1.2.3-a..b",
        "1.2.3-a_b",
        "1.2.3+a+b",
        "1.2.3+a_b",
        "1.2.3-é",
        "18446744073709551616.0.0",
    ] {
        assert!(Version::parse(text).is_err(), "should reject {text:?}");
    }
}

#[test]
fn error_names_the_input_and_reason() {
    let message = Version::parse("1.02.3").unwrap_err().to_string();
    assert!(message.contains("1.02.3") && message.contains("leading zero"), "{message}");
}

#[test]
fn precedence_follows_the_spec_example_chain() {
    let chain = [
        "1.0.0-alpha",
        "1.0.0-alpha.1",
        "1.0.0-alpha.beta",
        "1.0.0-beta",
        "1.0.0-beta.2",
        "1.0.0-beta.11",
        "1.0.0-rc.1",
        "1.0.0",
        "1.0.1",
        "1.1.0",
        "2.0.0",
    ];
    for pair in chain.windows(2) {
        assert!(v(pair[0]) < v(pair[1]), "{} < {}", pair[0], pair[1]);
    }
}

#[test]
fn numeric_identifiers_compare_as_numbers_and_below_alpha() {
    assert!(v("1.0.0-2") < v("1.0.0-10"));
    assert!(v("1.0.0-999") < v("1.0.0-a"));
    assert!(v("1.0.0-Z") < v("1.0.0-a"), "ASCII order puts capitals first");
    assert!(v("1.0.0-0") < v("1.0.0-0.0"), "a longer tag wins when the prefix is equal");
    assert!(v("1.0.0-0") == Version::lowest_prerelease(1, 0, 0));
}

#[test]
fn build_metadata_is_ignored_for_equality_and_order() {
    assert_eq!(v("1.0.0+a"), v("1.0.0+b"));
    assert_eq!(v("1.0.0+a").cmp(&v("1.0.0")), std::cmp::Ordering::Equal);
    assert_eq!(v("1.0.0+a").without_build().to_string(), "1.0.0");
}

#[test]
fn release_bumps_reset_lower_parts() {
    let base = v("1.2.3+meta");
    assert_eq!(base.bump(&Bump::Major).unwrap().to_string(), "2.0.0");
    assert_eq!(base.bump(&Bump::Minor).unwrap().to_string(), "1.3.0");
    assert_eq!(base.bump(&Bump::Patch).unwrap().to_string(), "1.2.4");
}

#[test]
fn bumping_a_prerelease_releases_it_when_the_core_already_moved() {
    assert_eq!(v("1.2.3-rc.1").bump(&Bump::Patch).unwrap().to_string(), "1.2.3");
    assert_eq!(v("1.2.0-rc.1").bump(&Bump::Minor).unwrap().to_string(), "1.2.0");
    assert_eq!(v("1.2.3-rc.1").bump(&Bump::Minor).unwrap().to_string(), "1.3.0");
    assert_eq!(v("2.0.0-rc.1").bump(&Bump::Major).unwrap().to_string(), "2.0.0");
    assert_eq!(v("2.1.0-rc.1").bump(&Bump::Major).unwrap().to_string(), "3.0.0");
}

#[test]
fn bump_overflow_is_an_error_not_a_panic() {
    let top = u64::MAX;
    assert!(v(&format!("{top}.0.0")).bump(&Bump::Major).is_err());
    assert!(v(&format!("0.{top}.0")).bump(&Bump::Minor).is_err());
    assert!(v(&format!("0.0.{top}")).bump(&Bump::Patch).is_err());
}
