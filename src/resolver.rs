//! Backtracking dependency resolution.
//!
//! At each step the search picks the unassigned required package with the fewest
//! remaining candidates, tries those candidates in preference order and undoes
//! the choice on failure. The search is complete: it fails only after exhausting
//! every combination, or when the step budget runs out.

use crate::constraint::Constraint;
use crate::model::{Dependency, Manifest, Resolution, ResolvedPackage};
use crate::registry::{Registry, Release};
use crate::version::Version;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Debug, Clone)]
pub struct Options {
    /// Try lowest versions first instead of highest.
    pub prefer_lowest: bool,
    /// Let prereleases match in-bounds constraints for every package.
    pub prerelease_all: bool,
    /// Let prereleases match in-bounds constraints for these packages.
    pub prerelease_packages: BTreeSet<String>,
    /// Exact versions to force; these win over pins in the manifest.
    pub pins: BTreeMap<String, Version>,
    /// Versions to try first if still acceptable, normally from a lockfile.
    pub preferred: BTreeMap<String, Version>,
    /// Upper limit on versions tried before giving up.
    pub max_steps: u64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            prefer_lowest: false,
            prerelease_all: false,
            prerelease_packages: BTreeSet::new(),
            pins: BTreeMap::new(),
            preferred: BTreeMap::new(),
            max_steps: 100_000,
        }
    }
}

impl Options {
    /// Pinning a package to a prerelease counts as opting in for that package.
    pub fn allows_prerelease(&self, name: &str) -> bool {
        self.prerelease_all
            || self.prerelease_packages.contains(name)
            || self.pins.get(name).is_some_and(Version::is_prerelease)
    }

    /// These options with the manifest's pins added underneath their own.
    pub fn with_manifest_pins(&self, manifest: &Manifest) -> Options {
        let mut merged = self.clone();
        for (name, version) in &manifest.pins {
            merged.pins.entry(name.clone()).or_insert_with(|| version.clone());
        }
        merged
    }
}

/// Who imposed a requirement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    Root(String),
    Package(String, Version),
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Origin::Root(name) => write!(f, "{name} (root)"),
            Origin::Package(name, version) => write!(f, "{name} {version}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    pub constraint: Constraint,
    pub origin: Origin,
}

/// The conflict the search got deepest into before failing. Earlier failures
/// are usually consequences of the same clash seen with fewer decisions made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub package: String,
    /// True when the registry has never heard of `package`.
    pub unknown_package: bool,
    /// Every active requirement on `package` at the point of failure.
    pub requirements: Vec<Requirement>,
    pub pin: Option<Version>,
    /// Set when `package` had been selected and a later requirement rejected it.
    pub selected: Option<Version>,
    /// The package's releases, lowest first.
    pub available: Vec<Version>,
    pub allow_prerelease: bool,
    /// Decisions in force at the point of failure, in the order they were made.
    pub decisions: Vec<(String, Version)>,
    /// Every version the search tried and abandoned, per package.
    pub tried: BTreeMap<String, Vec<Version>>,
    pub steps: u64,
    pub backtracks: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// The search space is exhausted: no selection satisfies the requirements.
    Conflict(Box<Conflict>),
    /// The budget ran out, so a solution might still exist.
    BudgetExceeded { steps: u64, conflict: Option<Box<Conflict>> },
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolveError::Conflict(conflict) => write!(f, "{conflict}"),
            ResolveError::BudgetExceeded { steps, conflict } => {
                writeln!(f, "gave up after {steps} steps without finding a solution")?;
                if let Some(conflict) = conflict {
                    write!(f, "deepest conflict so far:\n{conflict}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for ResolveError {}

/// A successful search and how much work it took.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Solved {
    pub resolution: Resolution,
    pub steps: u64,
    pub backtracks: u64,
}

/// Resolves the manifest against the registry.
pub fn resolve(
    registry: &Registry,
    manifest: &Manifest,
    options: &Options,
) -> Result<Solved, ResolveError> {
    let options = options.with_manifest_pins(manifest);
    let mut search = Search {
        registry,
        options: &options,
        requirements: BTreeMap::new(),
        assigned: BTreeMap::new(),
        trail: Vec::new(),
        tried: BTreeMap::new(),
        deepest: None,
        steps: 0,
        backtracks: 0,
    };
    let origin = Origin::Root(manifest.name.clone());
    for dependency in &manifest.dependencies {
        search.require(dependency, &origin);
    }

    match search.run() {
        Ok(true) => Ok(Solved {
            resolution: search.selection(manifest),
            steps: search.steps,
            backtracks: search.backtracks,
        }),
        Ok(false) => match search.finish_conflict() {
            Some(conflict) => Err(ResolveError::Conflict(conflict)),
            None => unreachable!("a failed search always records a conflict"),
        },
        Err(BudgetHit) => Err(ResolveError::BudgetExceeded {
            steps: search.steps,
            conflict: search.finish_conflict(),
        }),
    }
}

struct BudgetHit;

struct Search<'a> {
    registry: &'a Registry,
    options: &'a Options,
    requirements: BTreeMap<String, Vec<Requirement>>,
    assigned: BTreeMap<String, Version>,
    trail: Vec<(String, Version)>,
    tried: BTreeMap<String, BTreeSet<Version>>,
    deepest: Option<Conflict>,
    steps: u64,
    backtracks: u64,
}

impl<'a> Search<'a> {
    fn require(&mut self, dependency: &Dependency, origin: &Origin) {
        self.requirements.entry(dependency.name.clone()).or_default().push(Requirement {
            constraint: dependency.constraint.clone(),
            origin: origin.clone(),
        });
    }

    /// Acceptable releases of a package, best first.
    fn candidates(&self, name: &str) -> Vec<&'a Release> {
        let requirements = self.requirements.get(name).map_or(&[][..], Vec::as_slice);
        let pin = self.options.pins.get(name);
        let allow_pre = self.options.allows_prerelease(name);
        let mut found: Vec<&'a Release> = self
            .registry
            .releases(name)
            .iter()
            .filter(|release| pin.is_none_or(|p| *p == release.version))
            .filter(|release| {
                requirements.iter().all(|r| r.constraint.matches_with(&release.version, allow_pre))
            })
            .collect();
        if !self.options.prefer_lowest {
            found.reverse();
        }
        if let Some(preferred) = self.options.preferred.get(name) {
            if let Some(index) = found.iter().position(|r| r.version == *preferred) {
                let release = found.remove(index);
                found.insert(0, release);
            }
        }
        found
    }

    /// The most constrained package still needing a version, ties broken by name.
    fn next_decision(&self) -> Option<(String, Vec<&'a Release>)> {
        let mut best: Option<(String, Vec<&'a Release>)> = None;
        for name in self.requirements.keys() {
            if self.assigned.contains_key(name) {
                continue;
            }
            let candidates = self.candidates(name);
            if best.as_ref().is_none_or(|(_, current)| candidates.len() < current.len()) {
                let exhausted = candidates.is_empty();
                best = Some((name.clone(), candidates));
                if exhausted {
                    break;
                }
            }
        }
        best
    }

    /// Returns `Ok(true)` once every required package has a version.
    fn run(&mut self) -> Result<bool, BudgetHit> {
        let Some((name, candidates)) = self.next_decision() else {
            return Ok(true);
        };
        if candidates.is_empty() {
            self.note_conflict(&name, None);
            return Ok(false);
        }
        for release in candidates {
            if self.steps >= self.options.max_steps {
                return Err(BudgetHit);
            }
            self.steps += 1;
            self.tried.entry(name.clone()).or_default().insert(release.version.clone());
            self.assigned.insert(name.clone(), release.version.clone());
            self.trail.push((name.clone(), release.version.clone()));

            let origin = Origin::Package(name.clone(), release.version.clone());
            for dependency in &release.dependencies {
                self.require(dependency, &origin);
            }
            if self.dependencies_fit(release) && self.run()? {
                return Ok(true);
            }

            for dependency in &release.dependencies {
                let remaining = self.requirements.get_mut(&dependency.name).map(|list| {
                    list.pop();
                    list.is_empty()
                });
                if remaining == Some(true) {
                    self.requirements.remove(&dependency.name);
                }
            }
            self.trail.pop();
            self.assigned.remove(&name);
            self.backtracks += 1;
        }
        Ok(false)
    }

    /// Checks the new requirements against packages that already have a version.
    fn dependencies_fit(&mut self, release: &Release) -> bool {
        for dependency in &release.dependencies {
            if let Some(selected) = self.assigned.get(&dependency.name).cloned() {
                let allow_pre = self.options.allows_prerelease(&dependency.name);
                if !dependency.constraint.matches_with(&selected, allow_pre) {
                    self.note_conflict(&dependency.name, Some(selected));
                    return false;
                }
            }
        }
        true
    }

    fn note_conflict(&mut self, package: &str, selected: Option<Version>) {
        if self.deepest.as_ref().is_some_and(|d| d.decisions.len() >= self.trail.len()) {
            return;
        }
        self.deepest = Some(Conflict {
            package: package.to_string(),
            unknown_package: !self.registry.contains(package),
            requirements: self.requirements.get(package).cloned().unwrap_or_default(),
            pin: self.options.pins.get(package).cloned(),
            selected,
            available: self.registry.releases(package).iter().map(|r| r.version.clone()).collect(),
            allow_prerelease: self.options.allows_prerelease(package),
            decisions: self.trail.clone(),
            tried: BTreeMap::new(),
            steps: 0,
            backtracks: 0,
        });
    }

    fn finish_conflict(&mut self) -> Option<Box<Conflict>> {
        let mut conflict = self.deepest.take()?;
        conflict.tried = std::mem::take(&mut self.tried)
            .into_iter()
            .map(|(name, versions)| (name, versions.into_iter().collect()))
            .collect();
        conflict.steps = self.steps;
        conflict.backtracks = self.backtracks;
        Some(Box::new(conflict))
    }

    fn selection(&self, manifest: &Manifest) -> Resolution {
        let packages = self
            .assigned
            .iter()
            .map(|(name, version)| {
                let dependencies = self
                    .registry
                    .release(name, version)
                    .map(|release| release.dependencies.clone())
                    .unwrap_or_default();
                (name.clone(), ResolvedPackage { version: version.clone(), dependencies })
            })
            .collect();
        Resolution {
            root_name: manifest.name.clone(),
            root_version: manifest.version.clone(),
            root_dependencies: manifest.dependencies.clone(),
            packages,
        }
    }
}
