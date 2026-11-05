mod common;

use common::{c, v};
use semver_resolver::{Manifest, Registry, Release};

#[test]
fn registry_loads_sorted_with_sorted_dependencies() {
    let reg = Registry::from_json(
        r#"{"packages": {
            "a": {
                "2.0.0": {"dependencies": {"z": "^1", "b": "~2"}},
                "1.0.0-rc.1": {},
                "1.0.0": {}
            },
            "b": {}
        }}"#,
    )
    .unwrap();
    let order: Vec<String> = reg.releases("a").iter().map(|r| r.version.to_string()).collect();
    assert_eq!(order, ["1.0.0-rc.1", "1.0.0", "2.0.0"]);
    let names: Vec<&str> = reg.releases("a")[2].dependencies.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["b", "z"]);
    assert_eq!(reg.package_names().collect::<Vec<_>>(), ["a", "b"]);
    assert!(reg.contains("b"));
    assert!(reg.releases("b").is_empty());
    assert!(!reg.contains("nope"));
    assert!(reg.releases("nope").is_empty());
    assert!(reg.release("a", &v("1.0.0")).is_some());
    assert!(reg.release("a", &v("9.0.0")).is_none());
}

#[test]
fn registry_errors_point_at_the_offending_value() {
    let table = [
        (r#"[]"#, "registry: missing object field `packages`"),
        (r#"{"packages": 3}"#, "registry: missing object field `packages`"),
        (r#"{"packages": {"a": 3}}"#, "packages.a: expected an object of versions"),
        (r#"{"packages": {"a b": {}}}"#, "packages.a b: invalid package name"),
        (r#"{"packages": {"": {}}}"#, "invalid package name"),
        (r#"{"packages": {"a=b": {}}}"#, "invalid package name"),
        (r#"{"packages": {"a": {"1.x": {}}}}"#, "packages.a.1.x: invalid version"),
        (r#"{"packages": {"a": {"1.0.0": 3}}}"#, "packages.a.1.0.0: expected an object"),
        (
            r#"{"packages": {"a": {"1.0.0": {"depends": {}}}}}"#,
            "packages.a.1.0.0: unknown field `depends`",
        ),
        (
            r#"{"packages": {"a": {"1.0.0": {"dependencies": []}}}}"#,
            "packages.a.1.0.0.dependencies: expected an object",
        ),
        (
            r#"{"packages": {"a": {"1.0.0": {"dependencies": {"b": 1}}}}}"#,
            "packages.a.1.0.0.dependencies.b: expected a constraint string",
        ),
        (
            r#"{"packages": {"a": {"1.0.0": {"dependencies": {"b": "^^"}}}}}"#,
            "packages.a.1.0.0.dependencies.b: invalid constraint",
        ),
        (
            r#"{"packages": {"a": {"1.0.0+x": {}, "1.0.0+y": {}}}}"#,
            "packages.a.1.0.0+y: duplicate release",
        ),
        ("{", "invalid JSON: line 1, column 2"),
    ];
    for (text, expected) in table {
        let message = Registry::from_json(text).unwrap_err().to_string();
        assert!(message.contains(expected), "{text}: got {message}");
    }
}

#[test]
fn adding_a_duplicate_release_fails_and_keeps_the_original() {
    let mut reg = Registry::new();
    let release = |deps| Release { version: v("1.0.0"), dependencies: deps };
    reg.add("a", release(vec![])).unwrap();
    let message = reg.add("a", release(common::deps(&[("b", "*")]))).unwrap_err();
    assert!(message.contains("duplicate release 1.0.0 of `a`"), "{message}");
    assert!(reg.releases("a")[0].dependencies.is_empty());
}

#[test]
fn manifest_loads_all_fields() {
    let manifest = Manifest::from_json(
        r#"{"name": "app", "version": "1.2.3", "dependencies": {"b": "^1", "a": "2"}, "pins": {"c": "1.0.0"}}"#,
    )
    .unwrap();
    assert_eq!(manifest.name, "app");
    assert_eq!(manifest.version, Some(v("1.2.3")));
    assert_eq!(manifest.dependencies[0].name, "a");
    assert_eq!(manifest.dependencies[1].constraint, c("^1"));
    assert_eq!(manifest.pins["c"], v("1.0.0"));
}

#[test]
fn manifest_needs_only_a_name() {
    let manifest = Manifest::from_json(r#"{"name": "app"}"#).unwrap();
    assert_eq!(manifest.version, None);
    assert!(manifest.dependencies.is_empty());
    assert!(manifest.pins.is_empty());
}

#[test]
fn manifest_errors_point_at_the_offending_value() {
    let table = [
        (r#"{}"#, "manifest: missing string field `name`"),
        (r#"{"name": 1}"#, "manifest: missing string field `name`"),
        (r#"{"name": "a b"}"#, "manifest.name: invalid package name"),
        (r#"{"name": "a", "version": "1"}"#, "manifest.version: invalid version"),
        (r#"{"name": "a", "version": 1}"#, "manifest.version: expected a version string"),
        (r#"{"name": "a", "pins": []}"#, "manifest.pins: expected an object"),
        (r#"{"name": "a", "pins": {"b": 1}}"#, "manifest.pins.b: expected a version string"),
        (r#"{"name": "a", "pins": {"b": "^1"}}"#, "manifest.pins.b: invalid version"),
        (r#"{"name": "a", "dependencies": {"b": "1.x.1"}}"#, "manifest.dependencies.b: invalid"),
    ];
    for (text, expected) in table {
        let message = Manifest::from_json(text).unwrap_err().to_string();
        assert!(message.contains(expected), "{text}: got {message}");
    }
}
