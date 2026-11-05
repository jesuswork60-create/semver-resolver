//! Data shared by the subsystems: dependencies, the root manifest and the
//! resolved package set, plus the error type for all JSON-backed input files.

use crate::constraint::Constraint;
use crate::json::{Json, JsonError};
use crate::version::Version;
use std::collections::BTreeMap;
use std::fmt;

/// Invalid content in a registry, manifest or lockfile; `context` is a path to
/// the offending value such as `packages.web.1.0.0.dependencies.log`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataError {
    pub context: String,
    pub message: String,
}

impl DataError {
    pub(crate) fn new(context: impl Into<String>, message: impl fmt::Display) -> Self {
        DataError { context: context.into(), message: message.to_string() }
    }
}

impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.context, self.message)
    }
}

impl std::error::Error for DataError {}

impl From<JsonError> for DataError {
    fn from(err: JsonError) -> Self {
        DataError::new("invalid JSON", err)
    }
}

/// A requirement on another package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    pub name: String,
    pub constraint: Constraint,
}

impl Dependency {
    pub fn new(name: impl Into<String>, constraint: Constraint) -> Self {
        Dependency { name: name.into(), constraint }
    }
}

/// The project being resolved. The optional `pins` force exact versions of
/// packages if they enter the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub name: String,
    pub version: Option<Version>,
    pub dependencies: Vec<Dependency>,
    pub pins: BTreeMap<String, Version>,
}

impl Manifest {
    pub fn from_json(text: &str) -> Result<Self, DataError> {
        let doc = Json::parse(text)?;
        let name = required_str(&doc, "name", "manifest")?;
        validate_name(name, "manifest.name")?;
        let version = optional_version(&doc, "manifest")?;
        let dependencies = match doc.get("dependencies") {
            Some(value) => parse_dependencies(value, "manifest.dependencies")?,
            None => Vec::new(),
        };
        let mut pins = BTreeMap::new();
        if let Some(value) = doc.get("pins") {
            let entries = value
                .as_object()
                .ok_or_else(|| DataError::new("manifest.pins", "expected an object"))?;
            for (package, pinned) in entries {
                let context = format!("manifest.pins.{package}");
                validate_name(package, &context)?;
                let text = pinned
                    .as_str()
                    .ok_or_else(|| DataError::new(context.as_str(), "expected a version string"))?;
                let parsed = Version::parse(text).map_err(|e| DataError::new(context, e))?;
                pins.insert(package.clone(), parsed);
            }
        }
        Ok(Manifest { name: name.to_string(), version, dependencies, pins })
    }
}

/// A package chosen by the resolver or read back from a lockfile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPackage {
    pub version: Version,
    pub dependencies: Vec<Dependency>,
}

/// A complete selection: one version per package, plus the root's own requirements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub root_name: String,
    pub root_version: Option<Version>,
    pub root_dependencies: Vec<Dependency>,
    pub packages: BTreeMap<String, ResolvedPackage>,
}

pub(crate) fn required_str<'a>(
    doc: &'a Json,
    key: &str,
    context: &str,
) -> Result<&'a str, DataError> {
    doc.get(key)
        .and_then(Json::as_str)
        .ok_or_else(|| DataError::new(context, format!("missing string field `{key}`")))
}

pub(crate) fn optional_version(doc: &Json, context: &str) -> Result<Option<Version>, DataError> {
    let Some(value) = doc.get("version") else {
        return Ok(None);
    };
    let context = format!("{context}.version");
    let text = value
        .as_str()
        .ok_or_else(|| DataError::new(context.as_str(), "expected a version string"))?;
    Version::parse(text).map(Some).map_err(|e| DataError::new(context, e))
}

/// Package names may not be empty or hold whitespace, `=` (the CLI's `name=version`
/// separator) or control characters.
pub(crate) fn validate_name(name: &str, context: &str) -> Result<(), DataError> {
    if name.is_empty() || name.chars().any(|c| c.is_whitespace() || c.is_control() || c == '=') {
        return Err(DataError::new(context, format!("invalid package name `{name}`")));
    }
    Ok(())
}

/// Reads a `{"name": "constraint"}` object into dependencies sorted by name.
pub(crate) fn parse_dependencies(
    value: &Json,
    context: &str,
) -> Result<Vec<Dependency>, DataError> {
    let entries =
        value.as_object().ok_or_else(|| DataError::new(context, "expected an object"))?;
    let mut dependencies = Vec::with_capacity(entries.len());
    for (name, spec) in entries {
        let path = format!("{context}.{name}");
        validate_name(name, &path)?;
        let text = spec
            .as_str()
            .ok_or_else(|| DataError::new(path.as_str(), "expected a constraint string"))?;
        let constraint = Constraint::parse(text).map_err(|e| DataError::new(path, e))?;
        dependencies.push(Dependency::new(name.clone(), constraint));
    }
    dependencies.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(dependencies)
}

/// Writes dependencies back in the same object form, using the original spelling.
pub(crate) fn dependencies_to_json(dependencies: &[Dependency]) -> Json {
    Json::Object(
        dependencies
            .iter()
            .map(|d| (d.name.clone(), Json::string(d.constraint.source())))
            .collect(),
    )
}
