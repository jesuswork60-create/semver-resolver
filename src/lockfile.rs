//! The lockfile: a resolution saved as JSON, and checks of one against its inputs.
//!
//! Package entries are written in sorted order and constraints in their original
//! spelling, so a re-resolve that changes nothing produces an identical file.

use crate::graph;
use crate::json::Json;
use crate::model::{
    dependencies_to_json, optional_version, parse_dependencies, required_str, validate_name,
    DataError, Dependency, Manifest, Resolution, ResolvedPackage,
};
use crate::registry::Registry;
use crate::resolver::Options;
use crate::version::Version;
use std::collections::BTreeMap;
use std::fmt;

const FORMAT_VERSION: f64 = 1.0;

pub fn write(resolution: &Resolution) -> String {
    let mut root = vec![("name".to_string(), Json::string(resolution.root_name.as_str()))];
    if let Some(version) = &resolution.root_version {
        root.push(("version".to_string(), Json::string(version.to_string())));
    }
    root.push(("dependencies".to_string(), dependencies_to_json(&resolution.root_dependencies)));

    let packages = resolution
        .packages
        .iter()
        .map(|(name, package)| {
            let entry = Json::Object(vec![
                ("version".to_string(), Json::string(package.version.to_string())),
                ("dependencies".to_string(), dependencies_to_json(&package.dependencies)),
            ]);
            (name.clone(), entry)
        })
        .collect();

    Json::Object(vec![
        ("lockfile_version".to_string(), Json::Number(FORMAT_VERSION)),
        ("root".to_string(), Json::Object(root)),
        ("packages".to_string(), Json::Object(packages)),
    ])
    .pretty()
}

pub fn read(text: &str) -> Result<Resolution, DataError> {
    let doc = Json::parse(text)?;
    match doc.get("lockfile_version").and_then(Json::as_f64) {
        Some(found) if found == FORMAT_VERSION => {}
        Some(found) => {
            return Err(DataError::new("lockfile_version", format!("unsupported version {found}")));
        }
        None => return Err(DataError::new("lockfile", "missing number `lockfile_version`")),
    }

    let root = doc.get("root").ok_or_else(|| DataError::new("lockfile", "missing `root`"))?;
    let root_name = required_str(root, "name", "root")?.to_string();
    let root_version = optional_version(root, "root")?;
    let root_dependencies = dependencies_of(root, "root")?;

    let entries = doc
        .get("packages")
        .and_then(Json::as_object)
        .ok_or_else(|| DataError::new("lockfile", "missing object `packages`"))?;
    let mut packages = BTreeMap::new();
    for (name, entry) in entries {
        let context = format!("packages.{name}");
        validate_name(name, &context)?;
        let version = optional_version(entry, &context)?
            .ok_or_else(|| DataError::new(context.as_str(), "missing `version`"))?;
        let dependencies = dependencies_of(entry, &context)?;
        packages.insert(name.clone(), ResolvedPackage { version, dependencies });
    }
    Ok(Resolution { root_name, root_version, root_dependencies, packages })
}

fn dependencies_of(entry: &Json, context: &str) -> Result<Vec<Dependency>, DataError> {
    match entry.get("dependencies") {
        Some(value) => parse_dependencies(value, &format!("{context}.dependencies")),
        None => Ok(Vec::new()),
    }
}

/// The versions in a lockfile, for use as `Options::preferred`.
pub fn preferred_versions(resolution: &Resolution) -> BTreeMap<String, Version> {
    resolution
        .packages
        .iter()
        .map(|(name, package)| (name.clone(), package.version.clone()))
        .collect()
}

/// A way a lockfile disagrees with the manifest or the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockProblem {
    /// A requirement names a package the lockfile lacks.
    Missing { package: String, required_by: String },
    /// The locked version does not satisfy a requirement.
    Unsatisfied { package: String, version: Version, required_by: String, constraint: String },
    /// The locked version is not (or no longer) published.
    NotInRegistry { package: String, version: Version },
    /// The dependencies recorded for a package differ from the registry's.
    StaleDependencies { package: String, version: Version },
    /// The root's recorded requirements differ from the manifest's.
    StaleRoot,
    /// A pin is not honoured by the locked version.
    PinViolated { package: String, pinned: Version, locked: Version },
    /// Nothing requires this package.
    Extraneous { package: String },
}

impl fmt::Display for LockProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LockProblem::Missing { package, required_by } => {
                write!(f, "{package} is required by {required_by} but is not locked")
            }
            LockProblem::Unsatisfied { package, version, required_by, constraint } => write!(
                f,
                "{package} {version} does not satisfy {constraint} required by {required_by}"
            ),
            LockProblem::NotInRegistry { package, version } => {
                write!(f, "{package} {version} is not in the registry")
            }
            LockProblem::StaleDependencies { package, version } => {
                write!(f, "{package} {version} has different dependencies in the registry")
            }
            LockProblem::StaleRoot => {
                write!(f, "the manifest's dependencies differ from those in the lockfile")
            }
            LockProblem::PinViolated { package, pinned, locked } => {
                write!(f, "{package} is pinned to {pinned} but locked at {locked}")
            }
            LockProblem::Extraneous { package } => {
                write!(f, "{package} is locked but nothing requires it")
            }
        }
    }
}

/// Lists every problem with a lockfile, in a stable order; empty means it is valid.
pub fn verify(
    locked: &Resolution,
    manifest: &Manifest,
    registry: &Registry,
    options: &Options,
) -> Vec<LockProblem> {
    let options = options.with_manifest_pins(manifest);
    let mut problems = Vec::new();

    if locked.root_dependencies != manifest.dependencies {
        problems.push(LockProblem::StaleRoot);
    }

    // The manifest is authoritative for root requirements, so a stale lock
    // still reports each root requirement it fails.
    let mut requirers: Vec<(String, &[Dependency])> =
        vec![(format!("{} (root)", manifest.name), manifest.dependencies.as_slice())];
    for (name, package) in &locked.packages {
        requirers.push((format!("{name} {}", package.version), package.dependencies.as_slice()));
    }
    for (required_by, dependencies) in &requirers {
        for dependency in *dependencies {
            let Some(target) = locked.packages.get(&dependency.name) else {
                problems.push(LockProblem::Missing {
                    package: dependency.name.clone(),
                    required_by: required_by.clone(),
                });
                continue;
            };
            let allow_pre = options.allows_prerelease(&dependency.name);
            if !dependency.constraint.matches_with(&target.version, allow_pre) {
                problems.push(LockProblem::Unsatisfied {
                    package: dependency.name.clone(),
                    version: target.version.clone(),
                    required_by: required_by.clone(),
                    constraint: dependency.constraint.source().to_string(),
                });
            }
        }
    }

    for (name, package) in &locked.packages {
        let version = package.version.clone();
        match registry.release(name, &package.version) {
            None => problems.push(LockProblem::NotInRegistry { package: name.clone(), version }),
            Some(release) if release.dependencies != package.dependencies => {
                problems.push(LockProblem::StaleDependencies { package: name.clone(), version });
            }
            Some(_) => {}
        }
        if let Some(pinned) = options.pins.get(name).filter(|p| **p != package.version) {
            problems.push(LockProblem::PinViolated {
                package: name.clone(),
                pinned: pinned.clone(),
                locked: package.version.clone(),
            });
        }
        if graph::path_from_root(locked, name).is_none() {
            problems.push(LockProblem::Extraneous { package: name.clone() });
        }
    }
    problems
}
