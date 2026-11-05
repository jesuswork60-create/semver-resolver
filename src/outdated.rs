//! Compares a locked set with the registry to show what could be upgraded.

use crate::graph;
use crate::model::Resolution;
use crate::registry::Registry;
use crate::resolver::Options;
use crate::version::Version;

/// One package with a newer version available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutdatedRow {
    pub name: String,
    pub current: Version,
    /// The highest version every dependent in the locked graph still accepts.
    /// Considers those constraints only, not the new version's own dependencies.
    pub wanted: Version,
    /// The highest version published, ignoring constraints.
    pub latest: Version,
    /// True when the root requires this package itself.
    pub direct: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutdatedReport {
    pub rows: Vec<OutdatedRow>,
    /// Locked packages the registry has no releases for.
    pub unknown: Vec<String>,
}

/// Builds the report. Prereleases are not offered as `wanted` or `latest` unless
/// the options opt in, or the package is currently on a prerelease.
pub fn report(locked: &Resolution, registry: &Registry, options: &Options) -> OutdatedReport {
    let mut report = OutdatedReport::default();
    for (name, package) in &locked.packages {
        let current = &package.version;
        let allow_pre = options.allows_prerelease(name) || current.is_prerelease();
        let releases: Vec<&Version> = registry
            .releases(name)
            .iter()
            .map(|r| &r.version)
            .filter(|v| allow_pre || !v.is_prerelease())
            .collect();
        let Some(latest) = releases.last().copied() else {
            report.unknown.push(name.clone());
            continue;
        };

        let dependents = graph::dependents(locked, name);
        let wanted = releases
            .iter()
            .rev()
            .find(|v| dependents.iter().all(|(_, c)| c.matches_with(v, allow_pre)))
            .copied()
            .filter(|v| *v > current)
            .unwrap_or(current);

        if wanted > current || latest > current {
            report.rows.push(OutdatedRow {
                name: name.clone(),
                current: current.clone(),
                wanted: wanted.clone(),
                latest: latest.clone(),
                direct: locked.root_dependencies.iter().any(|d| d.name == *name),
            });
        }
    }
    report
}

impl OutdatedReport {
    pub fn to_table(&self) -> String {
        let mut out = String::new();
        if self.rows.is_empty() {
            out.push_str("everything is up to date\n");
        } else {
            let header = ["package", "current", "wanted", "latest"];
            let mut table = vec![header.map(String::from)];
            for row in &self.rows {
                let reach = if row.direct { "" } else { " (indirect)" };
                table.push([
                    format!("{}{reach}", row.name),
                    row.current.to_string(),
                    row.wanted.to_string(),
                    row.latest.to_string(),
                ]);
            }
            let mut widths = [0usize; 4];
            for cells in &table {
                for (width, cell) in widths.iter_mut().zip(cells) {
                    *width = (*width).max(cell.chars().count());
                }
            }
            for cells in &table {
                let line: Vec<String> = cells
                    .iter()
                    .zip(widths)
                    .map(|(cell, width)| format!("{cell:<width$}"))
                    .collect();
                out.push_str(line.join("  ").trim_end());
                out.push('\n');
            }
        }
        for name in &self.unknown {
            out.push_str(&format!("{name}: no releases in the registry\n"));
        }
        out
    }
}
