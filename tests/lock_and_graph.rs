mod common;

use common::{c, deps, manifest, registry, v};
use semver_resolver::explain::{explain_package, VersionStatus};
use semver_resolver::graph::{self, InstallStep};
use semver_resolver::lockfile::{self, LockProblem};
use semver_resolver::outdated;
use semver_resolver::{resolve, Options, Registry, Resolution, ResolvedPackage};
use std::collections::BTreeMap;

fn sample_registry() -> Registry {
    registry(&[
        ("a", "1.0.0", &[("b", "^1"), ("c", "^1")]),
        ("a", "1.1.0", &[("b", "^1"), ("c", "^1")]),
        ("a", "2.0.0", &[]),
        ("b", "1.0.0", &[("c", "~1.2")]),
        ("c", "1.2.0", &[]),
        ("c", "1.2.5", &[]),
        ("c", "1.9.0", &[]),
        ("d", "1.0.0", &[]),
    ])
}

fn sample_graph() -> Resolution {
    let project = manifest(&[("a", "^1"), ("d", "*")]);
    resolve(&sample_registry(), &project, &Options::default()).unwrap().resolution
}

fn graph_of(root: &[(&str, &str)], packages: &[(&str, &str, &[(&str, &str)])]) -> Resolution {
    let mut map = BTreeMap::new();
    for (name, version, dependencies) in packages {
        map.insert(
            name.to_string(),
            ResolvedPackage { version: v(version), dependencies: deps(dependencies) },
        );
    }
    Resolution {
        root_name: "app".into(),
        root_version: None,
        root_dependencies: deps(root),
        packages: map,
    }
}

#[test]
fn lockfile_round_trips() {
    let graph = sample_graph();
    let text = lockfile::write(&graph);
    let back = lockfile::read(&text).unwrap();
    assert_eq!(back, graph);
    assert_eq!(lockfile::write(&back), text, "writing is stable");
}

#[test]
fn lockfile_layout_is_exact_and_keeps_spelling() {
    let graph = graph_of(&[("x", ">=1.0, <2")], &[("x", "1.5.0", &[])]);
    let expected = r#"{
  "lockfile_version": 1,
  "root": {
    "name": "app",
    "dependencies": {
      "x": ">=1.0, <2"
    }
  },
  "packages": {
    "x": {
      "version": "1.5.0",
      "dependencies": {}
    }
  }
}
"#;
    assert_eq!(lockfile::write(&graph), expected);
}

#[test]
fn lockfile_read_rejects_bad_content() {
    let table = [
        (r#"{}"#, "missing number `lockfile_version`"),
        (r#"{"lockfile_version": 2}"#, "unsupported version 2"),
        (r#"{"lockfile_version": 1}"#, "missing `root`"),
        (r#"{"lockfile_version": 1, "root": {}}"#, "root: missing string field `name`"),
        (r#"{"lockfile_version": 1, "root": {"name": "a"}}"#, "missing object `packages`"),
        (
            r#"{"lockfile_version": 1, "root": {"name": "a"}, "packages": {"x": {}}}"#,
            "packages.x: missing `version`",
        ),
        (
            r#"{"lockfile_version": 1, "root": {"name": "a"}, "packages": {"x": {"version": "^1"}}}"#,
            "packages.x.version: invalid version",
        ),
        (
            r#"{"lockfile_version": 1, "root": {"name": "a", "dependencies": {"x": "??"}}, "packages": {}}"#,
            "root.dependencies.x: invalid constraint",
        ),
    ];
    for (text, expected) in table {
        let message = lockfile::read(text).unwrap_err().to_string();
        assert!(message.contains(expected), "{text}: got {message}");
    }
}

#[test]
fn re_resolving_with_a_lock_keeps_versions_until_they_stop_fitting() {
    let old = registry(&[("a", "1.0.0", &[("b", "^1")]), ("b", "1.0.0", &[])]);
    let project = manifest(&[("a", "^1")]);
    let first = resolve(&old, &project, &Options::default()).unwrap().resolution;

    let new = registry(&[
        ("a", "1.0.0", &[("b", "^1")]),
        ("a", "1.1.0", &[("b", "^1")]),
        ("b", "1.0.0", &[]),
        ("b", "1.1.0", &[]),
    ]);
    let locked = Options { preferred: lockfile::preferred_versions(&first), ..Options::default() };
    let again = resolve(&new, &project, &locked).unwrap().resolution;
    assert_eq!(lockfile::write(&again), lockfile::write(&first));

    let fresh = resolve(&new, &project, &Options::default()).unwrap().resolution;
    assert_eq!(fresh.packages["a"].version, v("1.1.0"));

    let tightened = manifest(&[("a", ">=1.1")]);
    let moved = resolve(&new, &tightened, &locked).unwrap().resolution;
    assert_eq!(moved.packages["a"].version, v("1.1.0"));
    assert_eq!(moved.packages["b"].version, v("1.0.0"), "unrelated lock entry still holds");
}

#[test]
fn verify_accepts_a_fresh_lock() {
    let project = manifest(&[("a", "^1"), ("d", "*")]);
    let problems = lockfile::verify(&sample_graph(), &project, &sample_registry(), &Options::default());
    assert_eq!(problems, vec![]);
}

#[test]
fn verify_reports_each_kind_of_problem() {
    let reg = registry(&[
        ("a", "1.0.0", &[("b", "^1")]),
        ("b", "1.0.0", &[]),
        ("b", "2.0.0", &[]),
        ("pinned", "1.0.0", &[]),
        ("pinned", "2.0.0", &[]),
        ("stray", "1.0.0", &[]),
    ]);
    let mut project = manifest(&[("a", "^1"), ("absent", "*"), ("pinned", "*")]);
    project.pins.insert("pinned".into(), v("1.0.0"));
    let locked = graph_of(
        &[("a", "^1")],
        &[
            ("a", "1.0.0", &[("b", "^1")]),
            ("b", "2.0.0", &[]),
            ("pinned", "2.0.0", &[]),
            ("stray", "1.0.0", &[("c", "^3")]),
            ("gone", "9.0.0", &[]),
        ],
    );
    let problems = lockfile::verify(&locked, &project, &reg, &Options::default());
    let has = |wanted: LockProblem| {
        assert!(problems.contains(&wanted), "missing {wanted:?} in {problems:?}");
    };
    has(LockProblem::StaleRoot);
    has(LockProblem::Missing { package: "absent".into(), required_by: "app (root)".into() });
    has(LockProblem::Missing { package: "c".into(), required_by: "stray 1.0.0".into() });
    has(LockProblem::Unsatisfied {
        package: "b".into(),
        version: v("2.0.0"),
        required_by: "a 1.0.0".into(),
        constraint: "^1".into(),
    });
    has(LockProblem::NotInRegistry { package: "gone".into(), version: v("9.0.0") });
    has(LockProblem::StaleDependencies { package: "stray".into(), version: v("1.0.0") });
    has(LockProblem::PinViolated {
        package: "pinned".into(),
        pinned: v("1.0.0"),
        locked: v("2.0.0"),
    });
    has(LockProblem::Extraneous { package: "gone".into() });
    assert!(problems.iter().all(|p| !p.to_string().is_empty()));
}

#[test]
fn verify_ignores_constraint_spelling_differences() {
    let reg = registry(&[("a", "1.0.0", &[])]);
    let project = manifest(&[("a", ">=1.0.0, <2.0.0-0")]);
    let locked = graph_of(&[("a", "^1.0.0")], &[("a", "1.0.0", &[])]);
    assert_eq!(lockfile::verify(&locked, &project, &reg, &Options::default()), vec![]);
}

#[test]
fn verify_respects_prerelease_opt_in() {
    let reg = registry(&[("a", "1.5.0-rc.1", &[])]);
    let project = manifest(&[("a", "^1")]);
    let locked = graph_of(&[("a", "^1")], &[("a", "1.5.0-rc.1", &[])]);
    assert_eq!(lockfile::verify(&locked, &project, &reg, &Options::default()).len(), 1);
    let opt_in = Options { prerelease_all: true, ..Options::default() };
    assert_eq!(lockfile::verify(&locked, &project, &reg, &opt_in), vec![]);
}

#[test]
fn tree_shows_structure_and_marks_repeats() {
    let expected = "\
app 0.1.0
├── a 1.1.0 [^1]
│   ├── b 1.0.0 [^1]
│   │   └── c 1.2.5 [~1.2]
│   └── c 1.2.5 [^1]
└── d 1.0.0 [*]
";
    assert_eq!(graph::render_tree(&sample_graph(), false), expected);
}

#[test]
fn tree_elides_repeated_subtrees_unless_asked_for_all() {
    let graph = graph_of(
        &[("a", "*"), ("b", "*")],
        &[("a", "1.0.0", &[("s", "*")]), ("b", "1.0.0", &[("s", "*")]), ("s", "1.0.0", &[("t", "*")]), ("t", "1.0.0", &[])],
    );
    let brief = graph::render_tree(&graph, false);
    assert!(brief.contains("└── s 1.0.0 [*] (*)\n"), "{brief}");
    assert_eq!(brief.matches("t 1.0.0").count(), 1, "{brief}");
    let full = graph::render_tree(&graph, true);
    assert!(!full.contains("(*)"), "{full}");
    assert_eq!(full.matches("t 1.0.0").count(), 2, "{full}");
}

#[test]
fn tree_stops_at_cycles_and_flags_missing_nodes() {
    let graph = graph_of(
        &[("a", "*"), ("ghost", "^1")],
        &[("a", "1.0.0", &[("b", "*")]), ("b", "1.0.0", &[("a", "^1")])],
    );
    for full in [false, true] {
        let text = graph::render_tree(&graph, full);
        assert!(text.contains("a 1.0.0 [^1] (cycle)\n"), "{text}");
        assert!(text.contains("└── ghost (missing) [^1]\n"), "{text}");
    }
}

fn position(order: &[InstallStep], name: &str) -> usize {
    order
        .iter()
        .position(|step| match step {
            InstallStep::Single(single) => single == name,
            InstallStep::Cycle(members) => members.iter().any(|m| m == name),
        })
        .unwrap_or_else(|| panic!("{name} missing from {order:?}"))
}

#[test]
fn install_order_puts_dependencies_first() {
    let graph = sample_graph();
    let order = graph::install_order(&graph);
    assert_eq!(order.len(), graph.packages.len());
    assert!(position(&order, "c") < position(&order, "b"));
    assert!(position(&order, "b") < position(&order, "a"));
    assert_eq!(order, graph::install_order(&graph), "deterministic");
}

#[test]
fn install_order_groups_cycles_and_orders_around_them() {
    let graph = graph_of(
        &[("top", "*")],
        &[
            ("top", "1.0.0", &[("x", "*")]),
            ("x", "1.0.0", &[("y", "*")]),
            ("y", "1.0.0", &[("z", "*"), ("leaf", "*")]),
            ("z", "1.0.0", &[("x", "*")]),
            ("leaf", "1.0.0", &[]),
            ("solo", "1.0.0", &[("solo", "*")]),
        ],
    );
    let order = graph::install_order(&graph);
    let cycle = InstallStep::Cycle(vec!["x".into(), "y".into(), "z".into()]);
    assert!(order.contains(&cycle), "{order:?}");
    assert!(order.contains(&InstallStep::Single("solo".into())), "self-dependency stays single");
    assert!(position(&order, "leaf") < position(&order, "x"));
    assert!(position(&order, "x") < position(&order, "top"));
}

#[test]
fn install_order_tolerates_missing_nodes() {
    let graph = graph_of(&[], &[("a", "1.0.0", &[("ghost", "*")])]);
    assert_eq!(graph::install_order(&graph), vec![InstallStep::Single("a".into())]);
}

#[test]
fn dependents_and_paths() {
    let graph = sample_graph();
    let who: Vec<String> = graph::dependents(&graph, "c").into_iter().map(|(w, _)| w).collect();
    assert_eq!(who, ["a 1.1.0", "b 1.0.0"]);
    let root_first = graph::dependents(&graph, "a");
    assert_eq!(root_first[0].0, "app (root)");
    assert_eq!(*root_first[0].1, c("^1"));

    assert_eq!(graph::path_from_root(&graph, "c").unwrap(), ["app", "a", "c"]);
    assert_eq!(graph::path_from_root(&graph, "d").unwrap(), ["app", "d"]);
    assert_eq!(graph::path_from_root(&graph, "nope"), None);

    let cyclic = graph_of(&[("a", "*")], &[("a", "1.0.0", &[("b", "*")]), ("b", "1.0.0", &[("a", "*")]), ("c", "1.0.0", &[("c", "*")])]);
    assert_eq!(graph::path_from_root(&cyclic, "b").unwrap(), ["app", "a", "b"]);
    assert_eq!(graph::path_from_root(&cyclic, "c"), None);
}

#[test]
fn explanation_classifies_each_version() {
    let reg = sample_registry();
    let graph = sample_graph();
    let explanation = explain_package(&graph, &reg, &Options::default(), "c").unwrap();
    assert_eq!(explanation.selected, v("1.2.5"));
    assert_eq!(explanation.path, ["app", "a", "c"]);
    assert_eq!(
        explanation.required_by,
        vec![("a 1.1.0".to_string(), "^1".to_string()), ("b 1.0.0".to_string(), "~1.2".to_string())]
    );
    let statuses: Vec<(String, VersionStatus)> =
        explanation.versions.iter().map(|(ver, s)| (ver.to_string(), s.clone())).collect();
    assert_eq!(statuses[1], ("1.2.5".to_string(), VersionStatus::Selected));
    assert_eq!(statuses[2], ("1.2.0".to_string(), VersionStatus::Allowed));
    assert_eq!(
        statuses[0],
        (
            "1.9.0".to_string(),
            VersionStatus::Rejected {
                by: "b 1.0.0".into(),
                constraint: "~1.2".into(),
                why: "out of range"
            }
        )
    );
    let text = explanation.to_string();
    assert!(text.contains("c 1.2.5 is selected"), "{text}");
    assert!(text.contains("reached through: app -> a -> c"), "{text}");
    assert!(text.contains("1.9.0  rejected by ~1.2 from b 1.0.0"), "{text}");
    assert!(text.contains("1.2.0  allowed, not chosen"), "{text}");
}

#[test]
fn explanation_covers_pins_prereleases_and_absent_packages() {
    let reg = registry(&[("a", "1.0.0", &[]), ("a", "1.1.0", &[]), ("a", "1.2.0-rc.1", &[])]);
    let mut options = Options::default();
    options.pins.insert("a".into(), v("1.0.0"));
    let graph = resolve(&reg, &manifest(&[("a", "^1")]), &options).unwrap().resolution;

    let pinned = explain_package(&graph, &reg, &options, "a").unwrap();
    assert_eq!(pinned.versions[1].1, VersionStatus::NotPinned);

    let unpinned = explain_package(&graph, &reg, &Options::default(), "a").unwrap();
    assert!(matches!(
        unpinned.versions[0].1,
        VersionStatus::Rejected { why: "prerelease not opted in", .. }
    ));
    assert!(explain_package(&graph, &reg, &options, "other").is_none());
}

#[test]
fn outdated_separates_wanted_from_latest() {
    let reg = sample_registry();
    let locked = graph_of(
        &[("a", "^1")],
        &[
            ("a", "1.0.0", &[("c", "~1.2")]),
            ("c", "1.2.0", &[]),
            ("d", "1.0.0", &[]),
            ("mystery", "1.0.0", &[]),
        ],
    );
    let report = outdated::report(&locked, &reg, &Options::default());
    assert_eq!(report.unknown, ["mystery"]);
    assert_eq!(report.rows.len(), 2);

    let a = &report.rows[0];
    assert_eq!((a.name.as_str(), a.direct), ("a", true));
    assert_eq!((a.current.clone(), a.wanted.clone(), a.latest.clone()), (v("1.0.0"), v("1.1.0"), v("2.0.0")));

    let dep = &report.rows[1];
    assert_eq!((dep.name.as_str(), dep.direct), ("c", false));
    assert_eq!((dep.wanted.clone(), dep.latest.clone()), (v("1.2.5"), v("1.9.0")));

    let table = report.to_table();
    assert!(table.starts_with("package"), "{table}");
    assert!(table.contains("c (indirect)"), "{table}");
    assert!(table.contains("mystery: no releases in the registry"), "{table}");
}

#[test]
fn outdated_says_so_when_nothing_is_newer() {
    let reg = registry(&[("a", "1.0.0", &[])]);
    let locked = graph_of(&[("a", "*")], &[("a", "1.0.0", &[])]);
    let report = outdated::report(&locked, &reg, &Options::default());
    assert!(report.rows.is_empty());
    assert_eq!(report.to_table(), "everything is up to date\n");
}

#[test]
fn outdated_ignores_prereleases_unless_opted_in_or_already_on_one() {
    let reg = registry(&[("a", "1.0.0", &[]), ("a", "1.1.0-rc.1", &[])]);
    let locked = graph_of(&[("a", "*")], &[("a", "1.0.0", &[])]);
    assert!(outdated::report(&locked, &reg, &Options::default()).rows.is_empty());

    let opt_in = Options { prerelease_all: true, ..Options::default() };
    let rows = outdated::report(&locked, &reg, &opt_in).rows;
    assert_eq!(rows[0].latest, v("1.1.0-rc.1"));
    assert_eq!(rows[0].wanted, v("1.1.0-rc.1"));

    let reg = registry(&[("a", "1.1.0-rc.1", &[]), ("a", "1.1.0-rc.2", &[])]);
    let on_pre = graph_of(&[("a", ">=1.1.0-rc.1")], &[("a", "1.1.0-rc.1", &[])]);
    let rows = outdated::report(&on_pre, &reg, &Options::default()).rows;
    assert_eq!(rows[0].wanted, v("1.1.0-rc.2"));
}

#[test]
fn outdated_never_offers_a_downgrade_as_wanted() {
    let reg = registry(&[("a", "1.0.0", &[]), ("a", "2.0.0", &[])]);
    let locked = graph_of(&[("a", "^1")], &[("a", "3.0.0", &[])]);
    let report = outdated::report(&locked, &reg, &Options::default());
    assert!(report.rows.is_empty(), "{:?}", report.rows);
}
