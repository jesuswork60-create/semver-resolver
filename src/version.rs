//! SemVer 2.0 versions: strict parsing, precedence and bumping.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

/// One dot-separated part of a prerelease tag.
///
/// The derived ordering is exactly the SemVer rule: numeric identifiers sort
/// numerically and below alphanumeric ones, which sort in ASCII order.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Identifier {
    Numeric(u64),
    Alpha(String),
}

impl fmt::Display for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Identifier::Numeric(n) => write!(f, "{n}"),
            Identifier::Alpha(s) => f.write_str(s),
        }
    }
}

/// A parsed version. Equality, ordering and hashing all ignore build metadata,
/// so `1.0.0+a == 1.0.0+b`; the metadata is only kept for display.
#[derive(Debug, Clone)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Vec<Identifier>,
    pub build: Vec<String>,
}

/// Which part of a version to increment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bump {
    Major,
    Minor,
    Patch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionError {
    pub input: String,
    pub reason: String,
}

impl fmt::Display for VersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid version `{}`: {}", self.input, self.reason)
    }
}

impl std::error::Error for VersionError {}

impl Version {
    pub fn new(major: u64, minor: u64, patch: u64) -> Self {
        Version { major, minor, patch, pre: Vec::new(), build: Vec::new() }
    }

    /// The lowest version that can exist with this core: `major.minor.patch-0`.
    pub fn lowest_prerelease(major: u64, minor: u64, patch: u64) -> Self {
        Version { pre: vec![Identifier::Numeric(0)], ..Version::new(major, minor, patch) }
    }

    pub fn parse(input: &str) -> Result<Self, VersionError> {
        let fail = |reason: String| VersionError { input: input.to_string(), reason };

        let (rest, build) = match input.split_once('+') {
            Some((rest, build)) => (rest, parse_build(build).map_err(fail)?),
            None => (input, Vec::new()),
        };
        let (core, pre) = match rest.split_once('-') {
            Some((core, pre)) => (core, parse_prerelease(pre).map_err(fail)?),
            None => (rest, Vec::new()),
        };

        let parts: Vec<&str> = core.split('.').collect();
        let [major, minor, patch] = parts.as_slice() else {
            return Err(fail("expected major.minor.patch".to_string()));
        };
        Ok(Version {
            major: parse_numeric(major).map_err(fail)?,
            minor: parse_numeric(minor).map_err(fail)?,
            patch: parse_numeric(patch).map_err(fail)?,
            pre,
            build,
        })
    }

    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }

    /// The `(major, minor, patch)` triple, without prerelease or build.
    pub fn core(&self) -> (u64, u64, u64) {
        (self.major, self.minor, self.patch)
    }

    pub fn without_build(&self) -> Self {
        Version { build: Vec::new(), ..self.clone() }
    }

    /// Returns the next version of the requested kind, dropping build metadata.
    ///
    /// Releasing a prerelease keeps the core when the core already reflects
    /// the bump: `1.2.0-rc.1` bumped by minor becomes `1.2.0`, not `1.3.0`.
    pub fn bump(&self, kind: &Bump) -> Result<Self, VersionError> {
        let overflow = || VersionError {
            input: self.to_string(),
            reason: "version component overflow while bumping".to_string(),
        };
        let inc = |n: u64| n.checked_add(1).ok_or_else(overflow);
        let pre = self.is_prerelease();
        let next = match kind {
            Bump::Major if pre && self.minor == 0 && self.patch == 0 => {
                Version::new(self.major, 0, 0)
            }
            Bump::Major => Version::new(inc(self.major)?, 0, 0),
            Bump::Minor if pre && self.patch == 0 => Version::new(self.major, self.minor, 0),
            Bump::Minor => Version::new(self.major, inc(self.minor)?, 0),
            Bump::Patch if pre => Version::new(self.major, self.minor, self.patch),
            Bump::Patch => Version::new(self.major, self.minor, inc(self.patch)?),
        };
        Ok(next)
    }
}

/// Parses a numeric component: digits only, no leading zero, fits in `u64`.
pub(crate) fn parse_numeric(text: &str) -> Result<u64, String> {
    if text.is_empty() {
        return Err("empty numeric component".to_string());
    }
    if !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("`{text}` is not a number"));
    }
    if text.len() > 1 && text.starts_with('0') {
        return Err(format!("`{text}` has a leading zero"));
    }
    text.parse().map_err(|_| format!("`{text}` is too large"))
}

fn check_identifier(text: &str) -> Result<(), String> {
    if text.is_empty() {
        return Err("empty identifier".to_string());
    }
    if !text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err(format!("identifier `{text}` has characters outside [0-9A-Za-z-]"));
    }
    Ok(())
}

fn parse_prerelease(text: &str) -> Result<Vec<Identifier>, String> {
    text.split('.')
        .map(|part| {
            check_identifier(part)?;
            if part.bytes().all(|b| b.is_ascii_digit()) {
                parse_numeric(part).map(Identifier::Numeric)
            } else {
                Ok(Identifier::Alpha(part.to_string()))
            }
        })
        .collect()
}

fn parse_build(text: &str) -> Result<Vec<String>, String> {
    text.split('.')
        .map(|part| check_identifier(part).map(|()| part.to_string()))
        .collect()
}

impl FromStr for Version {
    type Err = VersionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Version::parse(s)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        for (i, id) in self.pre.iter().enumerate() {
            write!(f, "{}{id}", if i == 0 { '-' } else { '.' })?;
        }
        if !self.build.is_empty() {
            write!(f, "+{}", self.build.join("."))?;
        }
        Ok(())
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core().cmp(&other.core()).then_with(|| {
            // A version without a prerelease tag outranks one with a tag;
            // tagged versions compare field by field, shorter prefix first.
            match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => self.pre.cmp(&other.pre),
            }
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Version {}

impl Hash for Version {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.core().hash(state);
        self.pre.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_component_rules() {
        assert_eq!(parse_numeric("0"), Ok(0));
        assert_eq!(parse_numeric("120"), Ok(120));
        assert!(parse_numeric("").is_err());
        assert!(parse_numeric("01").is_err());
        assert!(parse_numeric("1a").is_err());
        assert!(parse_numeric("99999999999999999999999").is_err());
    }

    #[test]
    fn hash_agrees_with_eq_across_build_metadata() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(Version::parse("1.0.0+a").unwrap());
        assert!(set.contains(&Version::parse("1.0.0+b").unwrap()));
        assert!(!set.contains(&Version::parse("1.0.0-b").unwrap()));
    }
}
