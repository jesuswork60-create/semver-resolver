//! The set of published packages available to the resolver.

use crate::json::Json;
use crate::model::{parse_dependencies, validate_name, DataError, Dependency};
use crate::version::Version;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub dependencies: Vec<Dependency>,
}

/// Package name to releases, each list sorted ascending by precedence.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Registry {
    packages: BTreeMap<String, Vec<Release>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads the file format:
    /// `{"packages": {"name": {"1.0.0": {"dependencies": {"other": "^2"}}}}}`.
    ///
    /// Unknown keys in a release are an error, so a misspelled `dependencies`
    /// cannot silently produce a package with no requirements.
    pub fn from_json(text: &str) -> Result<Self, DataError> {
        let doc = Json::parse(text)?;
        let packages = doc
            .get("packages")
            .and_then(Json::as_object)
            .ok_or_else(|| DataError::new("registry", "missing object field `packages`"))?;

        let mut registry = Registry::new();
        for (name, releases) in packages {
            let context = format!("packages.{name}");
            validate_name(name, &context)?;
            let releases = releases
                .as_object()
                .ok_or_else(|| DataError::new(context.as_str(), "expected an object of versions"))?;
            // A package listed with no releases is still known to exist.
            registry.packages.entry(name.clone()).or_default();
            for (version, body) in releases {
                let path = format!("{context}.{version}");
                let parsed =
                    Version::parse(version).map_err(|e| DataError::new(path.as_str(), e))?;
                let fields = body
                    .as_object()
                    .ok_or_else(|| DataError::new(path.as_str(), "expected an object"))?;
                if let Some((key, _)) = fields.iter().find(|(key, _)| key != "dependencies") {
                    return Err(DataError::new(path, format!("unknown field `{key}`")));
                }
                let dependencies = match body.get("dependencies") {
                    Some(value) => parse_dependencies(value, &format!("{path}.dependencies"))?,
                    None => Vec::new(),
                };
                registry
                    .add(name, Release { version: parsed, dependencies })
                    .map_err(|message| DataError::new(path, message))?;
            }
        }
        Ok(registry)
    }

    /// Adds a release. Versions that differ only in build metadata have equal
    /// precedence and cannot be told apart, so they count as duplicates.
    pub fn add(&mut self, name: &str, mut release: Release) -> Result<(), String> {
        release.dependencies.sort_by(|a, b| a.name.cmp(&b.name));
        let releases = self.packages.entry(name.to_string()).or_default();
        match releases.binary_search_by(|r| r.version.cmp(&release.version)) {
            Ok(_) => Err(format!("duplicate release {} of `{name}`", release.version)),
            Err(slot) => {
                releases.insert(slot, release);
                Ok(())
            }
        }
    }

    pub fn contains(&self, name: &str) -> bool {
        self.packages.contains_key(name)
    }

    /// All releases of a package, lowest first; empty for an unknown package.
    pub fn releases(&self, name: &str) -> &[Release] {
        self.packages.get(name).map_or(&[], Vec::as_slice)
    }

    pub fn release(&self, name: &str, version: &Version) -> Option<&Release> {
        let releases = self.releases(name);
        releases
            .binary_search_by(|r| r.version.cmp(version))
            .ok()
            .map(|index| &releases[index])
    }

    pub fn package_names(&self) -> impl Iterator<Item = &str> {
        self.packages.keys().map(String::as_str)
    }
}
