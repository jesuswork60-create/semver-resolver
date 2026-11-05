//! Human-readable reasons: why a resolution failed and why a version was chosen.

use crate::constraint::Constraint;
use crate::graph;
use crate::model::Resolution;
use crate::registry::Registry;
use crate::resolver::{Conflict, Options, Requirement};
use crate::version::Version;
use std::fmt;

/// Longest version listing in a report; older versions are summarized.
const VERSION_LIST_LIMIT: usize = 12;

/// Says why one requirement turns a version down.
fn rejection(constraint: &Constraint, version: &Version, allow_pre: bool) -> Option<&'static str> {
    if constraint.matches_with(version, allow_pre) {
        None
    } else if constraint.matches_with(version, true) {
        Some("prerelease not opted in")
    } else {
        Some("out of range")
    }
}

fn first_rejection<'a>(
    requirements: &'a [Requirement],
    version: &Version,
    allow_pre: bool,
) -> Option<(&'a Requirement, &'static str)> {
    requirements
        .iter()
        .find_map(|r| rejection(&r.constraint, version, allow_pre).map(|why| (r, why)))
}

fn version_list(versions: &[Version]) -> String {
    versions.iter().map(Version::to_string).collect::<Vec<_>>().join(", ")
}

impl fmt::Display for Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.unknown_package {
            writeln!(f, "cannot resolve: package `{}` is not in the registry", self.package)?;
        } else if let Some(selected) = &self.selected {
            writeln!(
                f,
                "cannot resolve: `{}` {selected} was selected, but a later requirement rejects it",
                self.package
            )?;
        } else {
            writeln!(f, "cannot resolve: no version of `{}` meets every requirement", self.package)?;
        }

        writeln!(f, "\nrequirements on {}:", self.package)?;
        let width = self.requirements.iter().map(|r| r.constraint.source().len()).max().unwrap_or(0);
        for requirement in &self.requirements {
            writeln!(
                f,
                "  {:<width$}  from {}",
                requirement.constraint.source(),
                requirement.origin
            )?;
        }
        if let Some(pin) = &self.pin {
            writeln!(f, "  pinned to {pin}")?;
        }
        if self.requirements.len() > 1 {
            let combined = self
                .requirements
                .iter()
                .map(|r| r.constraint.clone())
                .reduce(|a, b| a.intersect(&b))
                .unwrap_or_else(Constraint::any);
            writeln!(f, "  combined: {combined}")?;
        }

        if !self.unknown_package {
            self.write_available(f)?;
        }

        if !self.decisions.is_empty() {
            let path: Vec<String> =
                self.decisions.iter().map(|(name, version)| format!("{name} {version}")).collect();
            writeln!(f, "\nselected when this failed: {}", path.join(", "))?;
        }
        if !self.tried.is_empty() {
            writeln!(f, "\ntried:")?;
            for (name, versions) in &self.tried {
                writeln!(f, "  {name}: {}", version_list(versions))?;
            }
        }
        writeln!(f, "\nsearch: {} steps, {} backtracks", self.steps, self.backtracks)
    }
}

impl Conflict {
    fn write_available(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.available.is_empty() {
            return writeln!(f, "\navailable versions: none");
        }
        writeln!(f, "\navailable versions:")?;
        for version in self.available.iter().rev().take(VERSION_LIST_LIMIT) {
            write!(f, "  {version}")?;
            if self.pin.as_ref().is_some_and(|pin| pin != version) {
                write!(f, "  not the pinned version")?;
            } else if let Some((requirement, why)) =
                first_rejection(&self.requirements, version, self.allow_prerelease)
            {
                write!(
                    f,
                    "  rejected by {} from {} ({why})",
                    requirement.constraint.source(),
                    requirement.origin
                )?;
            }
            writeln!(f)?;
        }
        if self.available.len() > VERSION_LIST_LIMIT {
            writeln!(f, "  ... and {} older", self.available.len() - VERSION_LIST_LIMIT)?;
        }
        Ok(())
    }
}

/// How one release of a package fared against the final selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionStatus {
    Selected,
    /// Meets every requirement, but was passed over by the search order, a
    /// lockfile preference or a conflict among its own dependencies.
    Allowed,
    Rejected { by: String, constraint: String, why: &'static str },
    NotPinned,
}

/// The answer to "why is this package at this version?".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageExplanation {
    pub name: String,
    pub selected: Version,
    /// Each dependent with the constraint it imposes.
    pub required_by: Vec<(String, String)>,
    /// One shortest chain from the root down to this package.
    pub path: Vec<String>,
    /// Newest first.
    pub versions: Vec<(Version, VersionStatus)>,
}

/// Explains a package in a solved graph, or `None` if it is not part of it.
pub fn explain_package(
    resolution: &Resolution,
    registry: &Registry,
    options: &Options,
    name: &str,
) -> Option<PackageExplanation> {
    let selected = resolution.packages.get(name)?.version.clone();
    let dependents = graph::dependents(resolution, name);
    let allow_pre = options.allows_prerelease(name);
    let pin = options.pins.get(name);

    let versions = registry
        .releases(name)
        .iter()
        .rev()
        .map(|release| {
            let version = &release.version;
            let rejected = dependents.iter().find_map(|(who, constraint)| {
                rejection(constraint, version, allow_pre).map(|why| VersionStatus::Rejected {
                    by: who.clone(),
                    constraint: constraint.source().to_string(),
                    why,
                })
            });
            let status = if *version == selected {
                VersionStatus::Selected
            } else if pin.is_some_and(|p| p != version) {
                VersionStatus::NotPinned
            } else {
                rejected.unwrap_or(VersionStatus::Allowed)
            };
            (version.clone(), status)
        })
        .collect();

    Some(PackageExplanation {
        name: name.to_string(),
        selected,
        required_by: dependents
            .iter()
            .map(|(who, constraint)| (who.clone(), constraint.source().to_string()))
            .collect(),
        path: graph::path_from_root(resolution, name).unwrap_or_default(),
        versions,
    })
}

impl fmt::Display for PackageExplanation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{} {} is selected", self.name, self.selected)?;
        if !self.path.is_empty() {
            writeln!(f, "reached through: {}", self.path.join(" -> "))?;
        }
        writeln!(f, "\nrequired by:")?;
        for (who, constraint) in &self.required_by {
            writeln!(f, "  {who} wants {constraint}")?;
        }
        if !self.versions.is_empty() {
            writeln!(f, "\nversions:")?;
        }
        for (version, status) in self.versions.iter().take(VERSION_LIST_LIMIT) {
            match status {
                VersionStatus::Selected => writeln!(f, "  {version}  selected")?,
                VersionStatus::Allowed => writeln!(f, "  {version}  allowed, not chosen")?,
                VersionStatus::NotPinned => writeln!(f, "  {version}  not the pinned version")?,
                VersionStatus::Rejected { by, constraint, why } => {
                    writeln!(f, "  {version}  rejected by {constraint} from {by} ({why})")?;
                }
            }
        }
        if self.versions.len() > VERSION_LIST_LIMIT {
            writeln!(f, "  ... and {} older", self.versions.len() - VERSION_LIST_LIMIT)?;
        }
        Ok(())
    }
}
