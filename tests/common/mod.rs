//! Compact builders shared by the integration tests.
#![allow(dead_code)]

use semver_resolver::{Constraint, Dependency, Manifest, Registry, Release, Version};
use std::collections::BTreeMap;

pub fn v(text: &str) -> Version {
    Version::parse(text).unwrap()
}

pub fn c(text: &str) -> Constraint {
    Constraint::parse(text).unwrap()
}

pub fn deps(list: &[(&str, &str)]) -> Vec<Dependency> {
    list.iter().map(|(name, spec)| Dependency::new(*name, c(spec))).collect()
}

/// Builds a registry from `(package, version, dependencies)` rows.
pub fn registry(rows: &[(&str, &str, &[(&str, &str)])]) -> Registry {
    let mut registry = Registry::new();
    for (name, version, dependencies) in rows {
        registry
            .add(name, Release { version: v(version), dependencies: deps(dependencies) })
            .unwrap();
    }
    registry
}

pub fn manifest(list: &[(&str, &str)]) -> Manifest {
    Manifest {
        name: "app".to_string(),
        version: Some(v("0.1.0")),
        dependencies: deps(list),
        pins: BTreeMap::new(),
    }
}
