mod common;

use common::{c, deps, manifest, registry, v};
use semver_resolver::{resolve, Manifest, Options, Origin, Registry, Release, ResolveError, Solved, Version};
use std::collections::BTreeMap;

fn solve(registry: &Registry, manifest: &Manifest, options: &Options) -> Solved {
    resolve(registry, manifest, options).unwrap_or_else(|e| panic!("should resolve:\n{e}"))
}

fn versions(solved: &Solved) -> BTreeMap<String, String> {
    solved
        .resolution
        .packages
        .iter()
        .map(|(name, package)| (name.clone(), package.version.to_string()))
        .collect()
}

fn expect(solved: &Solved, list: &[(&str, &str)]) {
    let wanted: BTreeMap<String, String> =
        list.iter().map(|(n, ver)| (n.to_string(), ver.to_string())).collect();
    assert_eq!(versions(solved), wanted);
}

fn conflict_of(result: Result<Solved, ResolveError>) -> Box<semver_resolver::Conflict> {
    match result {
        Err(ResolveError::Conflict(conflict)) => conflict,
        other => panic!("expected a conflict, got {other:?}"),
    }
}

#[test]
fn no_dependencies_is_an_empty_solution() {
    let solved = solve(&Registry::new(), &manifest(&[]), &Options::default());
    assert!(solved.resolution.packages.is_empty());
    assert_eq!(solved.steps, 0);
    assert_eq!(solved.resolution.root_name, "app");
}

#[test]
fn picks_the_highest_matching_versions_transitively() {
    let reg = registry(&[
        ("a", "1.0.0", &[("b", "^1")]),
        ("a", "1.2.0", &[("b", "^1")]),
        ("a", "2.0.0", &[]),
        ("b", "1.0.0", &[]),
        ("b", "1.5.0", &[]),
        ("b", "2.0.0", &[]),
    ]);
    let solved = solve(&reg, &manifest(&[("a", "^1")]), &Options::default());
    expect(&solved, &[("a", "1.2.0"), ("b", "1.5.0")]);
    assert_eq!(solved.backtracks, 0);
}

#[test]
fn prefer_lowest_flips_the_order() {
    let reg = registry(&[
        ("a", "1.0.0", &[("b", ">=1.1")]),
        ("a", "1.2.0", &[]),
        ("b", "1.0.0", &[]),
        ("b", "1.1.0", &[]),
        ("b", "1.5.0", &[]),
    ]);
    let options = Options { prefer_lowest: true, ..Options::default() };
    let solved = solve(&reg, &manifest(&[("a", "^1")]), &options);
    expect(&solved, &[("a", "1.0.0"), ("b", "1.1.0")]);
}

#[test]
fn backtracks_to_an_older_version_when_the_newest_conflicts() {
    let reg = registry(&[
        ("a", "1.0.0", &[("c", "^1")]),
        ("a", "2.0.0", &[("c", "^2")]),
        ("b", "1.0.0", &[("c", "^1")]),
        ("c", "1.0.0", &[]),
        ("c", "1.4.0", &[]),
        ("c", "2.0.0", &[]),
    ]);
    let solved = solve(&reg, &manifest(&[("a", "*"), ("b", "*")]), &Options::default());
    expect(&solved, &[("a", "1.0.0"), ("b", "1.0.0"), ("c", "1.4.0")]);
}

#[test]
fn undoing_a_choice_drops_the_packages_it_brought_in() {
    let reg = registry(&[
        ("a", "1.0.0", &[]),
        ("a", "2.0.0", &[("extra", "^1"), ("gone", "^9")]),
        ("extra", "1.0.0", &[]),
        ("gone", "1.0.0", &[]),
    ]);
    let solved = solve(&reg, &manifest(&[("a", "*")]), &Options::default());
    expect(&solved, &[("a", "1.0.0")]);
    assert!(solved.backtracks >= 1);
}

#[test]
fn a_late_requirement_can_reject_a_package_already_selected() {
    // `z` (2 candidates) is decided before `y` (3), then every `y` needs the older `z`.
    let reg = registry(&[
        ("x", "1.0.0", &[("z", "^1")]),
        ("y", "1.0.0", &[("z", "<1.1")]),
        ("y", "2.0.0", &[("z", "<1.1")]),
        ("y", "3.0.0", &[("z", "<1.1")]),
        ("z", "1.0.0", &[]),
        ("z", "1.1.0", &[]),
    ]);
    let solved = solve(&reg, &manifest(&[("x", "*"), ("y", "*")]), &Options::default());
    expect(&solved, &[("x", "1.0.0"), ("y", "3.0.0"), ("z", "1.0.0")]);
    assert!(solved.backtracks >= 3, "each y failed against z 1.1.0: {}", solved.backtracks);
}

#[test]
fn most_constrained_package_goes_first() {
    let reg = registry(&[
        ("big", "1.0.0", &[]),
        ("big", "2.0.0", &[]),
        ("big", "3.0.0", &[]),
        ("tiny", "1.0.0", &[("nothing", "^9")]),
        ("nothing", "1.0.0", &[]),
    ]);
    let conflict =
        conflict_of(resolve(&reg, &manifest(&[("big", "*"), ("tiny", "*")]), &Options::default()));
    assert_eq!(conflict.package, "nothing");
    assert!(!conflict.tried.contains_key("big"), "big should never be tried: {:?}", conflict.tried);
    assert_eq!(conflict.steps, 1);
}

#[test]
fn conflict_names_each_requirement_its_imposer_and_the_versions() {
    let reg = registry(&[
        ("web", "2.1.0", &[("log", "^1")]),
        ("web", "2.0.0", &[("log", "^1")]),
        ("db", "1.4.0", &[("log", ">=2")]),
        ("log", "1.3.0", &[]),
        ("log", "2.1.0", &[]),
    ]);
    let conflict =
        conflict_of(resolve(&reg, &manifest(&[("db", "^1"), ("web", "^2")]), &Options::default()));
    assert_eq!(conflict.package, "log");
    assert_eq!(conflict.available, vec![v("1.3.0"), v("2.1.0")]);
    assert_eq!(conflict.requirements.len(), 2);
    assert_eq!(conflict.requirements[0].origin, Origin::Package("db".into(), v("1.4.0")));
    assert_eq!(conflict.tried["web"], vec![v("2.0.0"), v("2.1.0")]);

    let text = conflict.to_string();
    for needle in [
        "`log` 2.1.0 was selected, but a later requirement rejects it",
        "^1   from web 2.",
        ">=2  from db 1.4.0",
        "combined: <none>",
        "2.1.0  rejected by ^1 from web",
        "1.3.0  rejected by >=2 from db 1.4.0",
        "selected when this failed: db 1.4.0, log 2.1.0, web",
        "web: 2.0.0, 2.1.0",
    ] {
        assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
    }
}

#[test]
fn conflict_without_a_selection_says_no_version_fits() {
    // `x` and `y` are decided before `zed`, so its requirements clash before it is picked.
    let reg = registry(&[
        ("x", "1.0.0", &[("zed", "^1")]),
        ("y", "1.0.0", &[("zed", "^2")]),
        ("zed", "1.0.0", &[]),
        ("zed", "2.0.0", &[]),
    ]);
    let conflict =
        conflict_of(resolve(&reg, &manifest(&[("x", "*"), ("y", "*")]), &Options::default()));
    assert_eq!(conflict.package, "zed");
    assert_eq!(conflict.selected, None);
    let text = conflict.to_string();
    assert!(text.contains("no version of `zed` meets every requirement"), "{text}");
    assert!(text.contains("combined: <none>"), "{text}");
    assert!(text.contains("selected when this failed: x 1.0.0, y 1.0.0"), "{text}");
}

#[test]
fn root_requirement_appears_as_the_root() {
    let reg = registry(&[("a", "1.0.0", &[])]);
    let conflict = conflict_of(resolve(&reg, &manifest(&[("a", "^5")]), &Options::default()));
    assert_eq!(conflict.requirements[0].origin, Origin::Root("app".into()));
    assert!(conflict.decisions.is_empty());
    let text = conflict.to_string();
    assert!(text.contains("from app (root)"), "{text}");
    assert!(!text.contains("selected when"), "{text}");
    assert!(!text.contains("combined"), "{text}");
}

#[test]
fn clash_with_a_selected_version_is_described_as_such() {
    let reg = registry(&[("a", "1.0.0", &[("b", "^2")]), ("b", "1.0.0", &[("a", "^1")])]);
    // Both have one candidate; `a` first by name, then b's constraint rejects
    // nothing yet: a's own requirement on b leaves no candidate.
    let conflict = conflict_of(resolve(&reg, &manifest(&[("a", "*")]), &Options::default()));
    assert_eq!(conflict.package, "b");

    let reg = registry(&[
        ("a", "1.0.0", &[("b", "^1")]),
        ("b", "1.0.0", &[("a", "^2")]),
        ("a", "2.0.0", &[("b", "^2")]),
    ]);
    let conflict = conflict_of(resolve(&reg, &manifest(&[("a", "^1")]), &Options::default()));
    assert_eq!(conflict.package, "a");
    assert_eq!(conflict.selected, Some(v("1.0.0")));
    let text = conflict.to_string();
    assert!(text.contains("`a` 1.0.0 was selected, but a later requirement"), "{text}");
    assert!(text.contains("^2  from b 1.0.0"), "{text}");
}

#[test]
fn unknown_package_is_reported_with_its_requirer() {
    let reg = registry(&[("a", "1.0.0", &[("ghost", "^1")])]);
    let conflict = conflict_of(resolve(&reg, &manifest(&[("a", "*")]), &Options::default()));
    assert!(conflict.unknown_package);
    assert_eq!(conflict.package, "ghost");
    let text = conflict.to_string();
    assert!(text.contains("package `ghost` is not in the registry"), "{text}");
    assert!(text.contains("from a 1.0.0"), "{text}");
    assert!(!text.contains("available versions"), "{text}");
}

#[test]
fn package_listed_without_releases_is_known_but_has_no_versions() {
    let reg = Registry::from_json(r#"{"packages": {"hollow": {}}}"#).unwrap();
    let conflict = conflict_of(resolve(&reg, &manifest(&[("hollow", "*")]), &Options::default()));
    assert!(!conflict.unknown_package);
    assert!(conflict.to_string().contains("available versions: none"));
}

#[test]
fn long_version_lists_are_summarized() {
    let rows: Vec<String> = (0..20).map(|n| format!("1.{n}.0")).collect();
    let mut reg = Registry::new();
    for row in &rows {
        reg.add("a", Release { version: v(row), dependencies: vec![] }).unwrap();
    }
    let text = conflict_of(resolve(&reg, &manifest(&[("a", "^2")]), &Options::default())).to_string();
    assert!(text.contains("1.19.0  rejected"), "{text}");
    assert!(!text.contains("1.7.0  rejected"), "{text}");
    assert!(text.contains("... and 8 older"), "{text}");
}

#[test]
fn dependency_cycles_resolve() {
    let reg = registry(&[
        ("a", "1.0.0", &[("b", "^1")]),
        ("b", "1.0.0", &[("c", "^1")]),
        ("c", "1.0.0", &[("a", "^1")]),
    ]);
    let solved = solve(&reg, &manifest(&[("a", "*")]), &Options::default());
    expect(&solved, &[("a", "1.0.0"), ("b", "1.0.0"), ("c", "1.0.0")]);
}

#[test]
fn a_package_may_depend_on_itself_if_it_agrees() {
    let reg = registry(&[("a", "1.0.0", &[("a", "^1")]), ("a", "2.0.0", &[("a", "^1")])]);
    let solved = solve(&reg, &manifest(&[("a", "*")]), &Options::default());
    expect(&solved, &[("a", "1.0.0")]);
}

#[test]
fn pins_select_versions_only_for_packages_in_the_graph() {
    let reg = registry(&[("a", "1.0.0", &[]), ("a", "1.5.0", &[]), ("other", "1.0.0", &[])]);
    let mut options = Options::default();
    options.pins.insert("a".into(), v("1.0.0"));
    options.pins.insert("other".into(), v("1.0.0"));
    let solved = solve(&reg, &manifest(&[("a", "^1")]), &options);
    expect(&solved, &[("a", "1.0.0")]);
}

#[test]
fn option_pins_override_manifest_pins() {
    let reg = registry(&[("a", "1.0.0", &[]), ("a", "1.1.0", &[]), ("a", "1.2.0", &[])]);
    let mut project = manifest(&[("a", "^1")]);
    project.pins.insert("a".into(), v("1.1.0"));
    expect(&solve(&reg, &project, &Options::default()), &[("a", "1.1.0")]);

    let mut options = Options::default();
    options.pins.insert("a".into(), v("1.0.0"));
    expect(&solve(&reg, &project, &options), &[("a", "1.0.0")]);
}

#[test]
fn pin_that_contradicts_a_requirement_is_a_conflict_that_mentions_it() {
    let reg = registry(&[("a", "1.0.0", &[]), ("a", "2.0.0", &[])]);
    let mut options = Options::default();
    options.pins.insert("a".into(), v("2.0.0"));
    let conflict = conflict_of(resolve(&reg, &manifest(&[("a", "^1")]), &options));
    let text = conflict.to_string();
    assert!(text.contains("pinned to 2.0.0"), "{text}");
    assert!(text.contains("1.0.0  not the pinned version"), "{text}");
    assert!(text.contains("2.0.0  rejected by ^1"), "{text}");
}

#[test]
fn pin_to_a_version_not_published_is_a_conflict() {
    let reg = registry(&[("a", "1.0.0", &[])]);
    let mut options = Options::default();
    options.pins.insert("a".into(), v("1.0.1"));
    assert!(resolve(&reg, &manifest(&[("a", "^1")]), &options).is_err());
}

#[test]
fn pinning_a_prerelease_opts_that_package_in() {
    let reg = registry(&[("a", "1.0.0", &[]), ("a", "1.5.0-rc.1", &[])]);
    let mut options = Options::default();
    options.pins.insert("a".into(), v("1.5.0-rc.1"));
    let solved = solve(&reg, &manifest(&[("a", "^1")]), &options);
    expect(&solved, &[("a", "1.5.0-rc.1")]);
}

#[test]
fn prereleases_are_skipped_by_default() {
    let reg = registry(&[("a", "1.0.0", &[]), ("a", "1.5.0-rc.1", &[]), ("a", "2.0.0-rc.1", &[])]);
    let solved = solve(&reg, &manifest(&[("a", "*")]), &Options::default());
    expect(&solved, &[("a", "1.0.0")]);
}

#[test]
fn only_prereleases_available_gives_a_conflict_that_says_why() {
    let reg = registry(&[("a", "1.5.0-rc.1", &[])]);
    let text = conflict_of(resolve(&reg, &manifest(&[("a", "*")]), &Options::default())).to_string();
    assert!(text.contains("1.5.0-rc.1  rejected by * from app (root) (prerelease not opted in)"), "{text}");
}

#[test]
fn a_requirement_naming_the_prerelease_core_can_select_it() {
    let reg = registry(&[("a", "1.0.0", &[]), ("a", "1.5.0-rc.1", &[]), ("a", "1.5.0-rc.2", &[])]);
    let solved = solve(&reg, &manifest(&[("a", ">=1.5.0-rc.1")]), &Options::default());
    expect(&solved, &[("a", "1.5.0-rc.2")]);
}

#[test]
fn prerelease_opt_in_applies_globally_or_per_package() {
    let reg = registry(&[
        ("a", "1.0.0", &[]),
        ("a", "1.5.0-rc.1", &[]),
        ("b", "1.0.0", &[]),
        ("b", "1.5.0-rc.1", &[]),
    ]);
    let project = manifest(&[("a", "^1"), ("b", "^1")]);

    let all = Options { prerelease_all: true, ..Options::default() };
    expect(&solve(&reg, &project, &all), &[("a", "1.5.0-rc.1"), ("b", "1.5.0-rc.1")]);

    let mut one = Options::default();
    one.prerelease_packages.insert("b".into());
    expect(&solve(&reg, &project, &one), &[("a", "1.0.0"), ("b", "1.5.0-rc.1")]);
}

#[test]
fn opt_in_also_applies_when_checking_selected_packages() {
    let reg = registry(&[
        ("a", "1.0.0", &[]),
        ("b", "1.0.0", &[("a", "^1")]),
        ("a", "1.5.0-rc.1", &[]),
    ]);
    let all = Options { prerelease_all: true, ..Options::default() };
    let solved = solve(&reg, &manifest(&[("a", "*"), ("b", "*")]), &all);
    expect(&solved, &[("a", "1.5.0-rc.1"), ("b", "1.0.0")]);
}

#[test]
fn preferred_versions_win_while_still_acceptable() {
    let reg = registry(&[("a", "1.0.0", &[]), ("a", "1.5.0", &[]), ("a", "2.0.0", &[])]);
    let mut options = Options::default();
    options.preferred.insert("a".into(), v("1.0.0"));
    expect(&solve(&reg, &manifest(&[("a", "*")]), &options), &[("a", "1.0.0")]);
    // The manifest moved on: the preference no longer applies.
    expect(&solve(&reg, &manifest(&[("a", ">=1.2")]), &options), &[("a", "2.0.0")]);
}

#[test]
fn preferred_version_also_beats_prefer_lowest() {
    let reg = registry(&[("a", "1.0.0", &[]), ("a", "1.5.0", &[])]);
    let mut options = Options { prefer_lowest: true, ..Options::default() };
    options.preferred.insert("a".into(), v("1.5.0"));
    expect(&solve(&reg, &manifest(&[("a", "*")]), &options), &[("a", "1.5.0")]);
}

#[test]
fn preference_is_abandoned_when_it_causes_a_conflict() {
    let reg = registry(&[
        ("a", "1.0.0", &[("c", "^1")]),
        ("a", "2.0.0", &[("c", "^2")]),
        ("c", "1.0.0", &[]),
        ("c", "2.0.0", &[]),
        ("b", "1.0.0", &[("c", "^2")]),
    ]);
    let mut options = Options::default();
    options.preferred.insert("a".into(), v("1.0.0"));
    let solved = solve(&reg, &manifest(&[("a", "*"), ("b", "*")]), &options);
    expect(&solved, &[("a", "2.0.0"), ("b", "1.0.0"), ("c", "2.0.0")]);
}

/// Each `layer-i` has two versions, and a shared package accepts only one
/// combination at the very end, which forces a walk through the search space.
fn exhausting_registry(layers: usize) -> (Registry, Manifest) {
    let mut reg = Registry::new();
    let mut root = Vec::new();
    for i in 0..layers {
        let name = format!("layer-{i}");
        for version in ["1.0.0", "2.0.0"] {
            let dependencies = deps(&[("tail", "^1")]);
            reg.add(&name, Release { version: v(version), dependencies }).unwrap();
        }
        root.push((name, "*"));
    }
    reg.add("tail", Release { version: v("1.0.0"), dependencies: deps(&[("sink", "^9")]) }).unwrap();
    reg.add("tail", Release { version: v("1.1.0"), dependencies: deps(&[("sink", "^9")]) }).unwrap();
    reg.add("sink", Release { version: v("1.0.0"), dependencies: vec![] }).unwrap();
    let list: Vec<(&str, &str)> = root.iter().map(|(n, s)| (n.as_str(), *s)).collect();
    (reg, manifest(&list))
}

#[test]
fn budget_stops_a_long_search_and_reports_what_it_saw() {
    let (reg, project) = exhausting_registry(12);
    let options = Options { max_steps: 50, ..Options::default() };
    match resolve(&reg, &project, &options) {
        Err(ResolveError::BudgetExceeded { steps, conflict }) => {
            assert_eq!(steps, 50);
            let conflict = conflict.expect("a conflict was seen before the budget ran out");
            assert_eq!(conflict.package, "sink");
            assert!(conflict.steps == 50);
        }
        other => panic!("expected budget exhaustion, got {other:?}"),
    }
}

#[test]
fn exhausting_the_budget_is_distinct_from_a_real_conflict() {
    let (reg, project) = exhausting_registry(3);
    assert!(matches!(
        resolve(&reg, &project, &Options::default()),
        Err(ResolveError::Conflict(_))
    ));
}

#[test]
fn zero_budget_is_exhausted_by_any_decision() {
    let reg = registry(&[("a", "1.0.0", &[])]);
    let options = Options { max_steps: 0, ..Options::default() };
    match resolve(&reg, &manifest(&[("a", "*")]), &options) {
        Err(error @ ResolveError::BudgetExceeded { steps: 0, conflict: None }) => {
            assert!(error.to_string().contains("gave up after 0 steps"));
        }
        other => panic!("expected budget exhaustion, got {other:?}"),
    }
    assert!(resolve(&reg, &manifest(&[]), &options).is_ok());
}

/// Small xorshift generator, so the randomized test below is reproducible.
struct Rng(u64);

impl Rng {
    fn below(&mut self, limit: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % limit
    }
}

const NAMES: [&str; 4] = ["p", "q", "r", "s"];
const VERSIONS: [&str; 3] = ["1.0.0", "1.1.0", "2.0.0"];
const SPECS: [&str; 5] = ["^1", "^2", "*", "=1.0.0", "<1.1"];

/// Tries all selections directly: is there a closed, consistent package set?
fn brute_force_solvable(reg: &Registry, project: &Manifest) -> bool {
    let choices = VERSIONS.len() as u64 + 1;
    for code in 0..choices.pow(NAMES.len() as u32) {
        let mut rest = code;
        let mut selection: BTreeMap<&str, Version> = BTreeMap::new();
        for name in NAMES {
            let pick = (rest % choices) as usize;
            rest /= choices;
            if pick > 0 {
                selection.insert(name, v(VERSIONS[pick - 1]));
            }
        }
        let fits = |list: &[semver_resolver::Dependency]| {
            list.iter().all(|d| {
                selection.get(d.name.as_str()).is_some_and(|sel| d.constraint.matches(sel))
            })
        };
        let closed = fits(&project.dependencies)
            && selection.iter().all(|(name, version)| {
                reg.release(name, version).is_some_and(|release| fits(&release.dependencies))
            });
        if closed {
            return true;
        }
    }
    false
}

#[test]
fn random_registries_agree_with_brute_force() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let (mut solved_count, mut failed_count) = (0, 0);
    for _ in 0..400 {
        let mut reg = Registry::new();
        for name in NAMES {
            for version in VERSIONS {
                if rng.below(4) == 0 {
                    continue;
                }
                let mut list = Vec::new();
                for other in NAMES {
                    if rng.below(4) == 0 {
                        list.push((other, SPECS[rng.below(5) as usize]));
                    }
                }
                reg.add(name, Release { version: v(version), dependencies: deps(&list) }).unwrap();
            }
        }
        let root = [
            (NAMES[rng.below(4) as usize], SPECS[rng.below(5) as usize]),
            (NAMES[rng.below(4) as usize], SPECS[rng.below(5) as usize]),
        ];
        let list: &[(&str, &str)] = if root[0].0 == root[1].0 { &root[..1] } else { &root[..] };
        let project = manifest(list);

        let expected = brute_force_solvable(&reg, &project);
        match resolve(&reg, &project, &Options::default()) {
            Ok(solved) => {
                assert!(expected, "resolver found a solution brute force did not");
                solved_count += 1;
                // Every selected package is consistent and reachable from the root.
                let packages = &solved.resolution.packages;
                for dep in &project.dependencies {
                    assert!(dep.constraint.matches(&packages[&dep.name].version));
                }
                for (name, package) in packages {
                    assert_eq!(
                        reg.release(name, &package.version).unwrap().dependencies,
                        package.dependencies
                    );
                    for dep in &package.dependencies {
                        assert!(dep.constraint.matches(&packages[&dep.name].version));
                    }
                    assert!(semver_resolver::graph::path_from_root(&solved.resolution, name).is_some());
                }
            }
            Err(ResolveError::Conflict(_)) => {
                assert!(!expected, "resolver missed a solution for {reg:?} with {list:?}");
                failed_count += 1;
            }
            Err(other) => panic!("unexpected: {other}"),
        }
    }
    assert!(solved_count > 40 && failed_count > 40, "weak mix: {solved_count} / {failed_count}");
}

#[test]
fn results_are_repeatable() {
    let reg = registry(&[
        ("a", "1.0.0", &[("c", "*")]),
        ("a", "1.1.0", &[("c", "<2")]),
        ("b", "1.0.0", &[("c", ">=1.5")]),
        ("c", "1.0.0", &[]),
        ("c", "1.5.0", &[]),
        ("c", "2.0.0", &[]),
    ]);
    let project = manifest(&[("a", "*"), ("b", "*")]);
    let first = solve(&reg, &project, &Options::default());
    assert_eq!(first, solve(&reg, &project, &Options::default()));
}

#[test]
fn a_package_named_like_the_root_is_a_separate_package() {
    let reg = registry(&[("app", "1.0.0", &[("b", "*")]), ("b", "1.0.0", &[("app", "^1")])]);
    let solved = solve(&reg, &manifest(&[("app", "^1")]), &Options::default());
    expect(&solved, &[("app", "1.0.0"), ("b", "1.0.0")]);
    assert_eq!(solved.resolution.root_name, "app");
    assert_eq!(solved.resolution.root_version, Some(v("0.1.0")));
}

#[test]
fn the_library_example_from_the_readme_works() {
    let registry = Registry::from_json(r#"{"packages": {"wick": {"0.3.5": {}}}}"#).unwrap();
    let manifest =
        Manifest::from_json(r#"{"name": "cabin", "dependencies": {"wick": "~0.3"}}"#).unwrap();
    let solved = resolve(&registry, &manifest, &Options::default()).unwrap();
    assert_eq!(solved.resolution.packages["wick"].version.to_string(), "0.3.5");
}

#[test]
fn constraint_helper_matches_library_parse() {
    assert_eq!(c("^1"), c(">=1.0.0 <2.0.0-0"));
}
