//! Version constraints compiled to sorted, merged intervals.
//!
//! Accepted syntax: `^ ~ ~> >= > <= < =`, bare versions, wildcards (`*`, `1.x`,
//! `1.2.*`), partial versions (`1`, `1.2`), hyphen ranges (`1.2 - 2.0`), commas
//! or spaces for AND, and `||` for OR. The empty string means `*`.
//!
//! Prereleases follow the npm rule. A prerelease version only matches an
//! interval if a comparator in the same AND group named a prerelease on the
//! same `major.minor.patch`, unless the caller opts in. Each interval therefore
//! carries the list of cores whose prereleases it admits.
//!
//! Upper bounds derived from `^`, `~` and x-ranges are written as `<2.0.0-0`,
//! which keeps `2.0.0-alpha` out even after opting in to prereleases. Lower
//! bounds derived from partial versions stay on the release, so `1.x` does not
//! admit `1.0.0-alpha` in opt-in mode.

use crate::version::{parse_numeric, Version};
use std::cmp::Ordering;
use std::fmt;

type Core = (u64, u64, u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bound {
    Unbounded,
    Inclusive(Version),
    Exclusive(Version),
}

impl Bound {
    fn version(&self) -> Option<&Version> {
        match self {
            Bound::Unbounded => None,
            Bound::Inclusive(v) | Bound::Exclusive(v) => Some(v),
        }
    }
}

/// Orders lower bounds from most to least permissive.
fn cmp_lower(a: &Bound, b: &Bound) -> Ordering {
    match (a, b) {
        (Bound::Unbounded, Bound::Unbounded) => Ordering::Equal,
        (Bound::Unbounded, _) => Ordering::Less,
        (_, Bound::Unbounded) => Ordering::Greater,
        (Bound::Inclusive(x), Bound::Exclusive(y)) if x == y => Ordering::Less,
        (Bound::Exclusive(x), Bound::Inclusive(y)) if x == y => Ordering::Greater,
        _ => a.version().cmp(&b.version()),
    }
}

/// Orders upper bounds from least to most permissive.
fn cmp_upper(a: &Bound, b: &Bound) -> Ordering {
    match (a, b) {
        (Bound::Unbounded, Bound::Unbounded) => Ordering::Equal,
        (Bound::Unbounded, _) => Ordering::Greater,
        (_, Bound::Unbounded) => Ordering::Less,
        (Bound::Exclusive(x), Bound::Inclusive(y)) if x == y => Ordering::Less,
        (Bound::Inclusive(x), Bound::Exclusive(y)) if x == y => Ordering::Greater,
        _ => a.version().cmp(&b.version()),
    }
}

/// True when an interval ending at `upper` overlaps or abuts one starting at `lower`.
fn touches(upper: &Bound, lower: &Bound) -> bool {
    match (upper, lower) {
        (Bound::Unbounded, _) | (_, Bound::Unbounded) => true,
        (Bound::Exclusive(u), Bound::Exclusive(l)) => u > l,
        (Bound::Inclusive(u) | Bound::Exclusive(u), Bound::Inclusive(l) | Bound::Exclusive(l)) => {
            u >= l
        }
    }
}

/// One contiguous range of versions plus the cores whose prereleases it admits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interval {
    pub lower: Bound,
    pub upper: Bound,
    pub prerelease_cores: Vec<Core>,
}

impl Interval {
    fn everything() -> Self {
        Interval { lower: Bound::Unbounded, upper: Bound::Unbounded, prerelease_cores: Vec::new() }
    }

    fn nothing() -> Self {
        Interval { upper: Bound::Exclusive(Version::lowest_prerelease(0, 0, 0)), ..Self::everything() }
    }

    fn within_bounds(&self, v: &Version) -> bool {
        let above = match &self.lower {
            Bound::Unbounded => true,
            Bound::Inclusive(l) => v >= l,
            Bound::Exclusive(l) => v > l,
        };
        let below = match &self.upper {
            Bound::Unbounded => true,
            Bound::Inclusive(u) => v <= u,
            Bound::Exclusive(u) => v < u,
        };
        above && below
    }

    fn admits(&self, v: &Version, include_prerelease: bool) -> bool {
        self.within_bounds(v)
            && (!v.is_prerelease()
                || include_prerelease
                || self.prerelease_cores.contains(&v.core()))
    }

    fn narrowed_bounds(&self, other: &Interval) -> (Bound, Bound) {
        let lower = if cmp_lower(&self.lower, &other.lower) == Ordering::Greater {
            self.lower.clone()
        } else {
            other.lower.clone()
        };
        let upper = if cmp_upper(&self.upper, &other.upper) == Ordering::Less {
            self.upper.clone()
        } else {
            other.upper.clone()
        };
        (lower, upper)
    }

    /// AND within one group: comparators pool their prerelease permissions.
    fn and(&self, other: &Interval) -> Interval {
        let (lower, upper) = self.narrowed_bounds(other);
        let mut prerelease_cores = self.prerelease_cores.clone();
        prerelease_cores.extend_from_slice(&other.prerelease_cores);
        Interval { lower, upper, prerelease_cores }
    }

    /// Intersection of two separate constraints: a prerelease must pass both.
    fn intersect(&self, other: &Interval) -> Interval {
        let (lower, upper) = self.narrowed_bounds(other);
        let prerelease_cores = self
            .prerelease_cores
            .iter()
            .filter(|core| other.prerelease_cores.contains(core))
            .copied()
            .collect();
        Interval { lower, upper, prerelease_cores }
    }

    fn is_void(&self) -> bool {
        if let Bound::Exclusive(u) = &self.upper {
            if *u == Version::lowest_prerelease(0, 0, 0) {
                return true;
            }
        }
        match (self.lower.version(), self.upper.version()) {
            (Some(l), Some(u)) => {
                l > u
                    || (l == u
                        && !(matches!(self.lower, Bound::Inclusive(_))
                            && matches!(self.upper, Bound::Inclusive(_))))
            }
            _ => false,
        }
    }

    /// True when some prerelease of `core` lies inside the bounds.
    fn holds_prereleases_of(&self, core: Core) -> bool {
        let lowest = Version::lowest_prerelease(core.0, core.1, core.2);
        let release = Version::new(core.0, core.1, core.2);
        let lower_ok = self.lower.version().is_none_or(|l| *l < release);
        let upper_ok = match &self.upper {
            Bound::Unbounded => true,
            Bound::Inclusive(u) => *u >= lowest,
            Bound::Exclusive(u) => *u > lowest,
        };
        lower_ok && upper_ok
    }

    /// Drops void intervals and permissions that cannot apply inside the bounds.
    fn normalized(mut self) -> Option<Interval> {
        if self.is_void() {
            return None;
        }
        let mut cores = std::mem::take(&mut self.prerelease_cores);
        cores.sort_unstable();
        cores.dedup();
        cores.retain(|core| self.holds_prereleases_of(*core));
        self.prerelease_cores = cores;
        Some(self)
    }

    /// The permissions a parser would derive from these bounds alone.
    fn implied_cores(&self) -> Vec<Core> {
        let mut cores: Vec<Core> = [&self.lower, &self.upper]
            .iter()
            .filter_map(|b| b.version())
            .filter(|v| v.is_prerelease())
            .map(Version::core)
            .filter(|core| self.holds_prereleases_of(*core))
            .collect();
        cores.sort_unstable();
        cores.dedup();
        cores
    }
}

impl fmt::Display for Interval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.lower, &self.upper) {
            (Bound::Unbounded, Bound::Unbounded) => f.write_str("*")?,
            (Bound::Inclusive(l), Bound::Inclusive(u)) if l == u => write!(f, "={l}")?,
            (lower, upper) => {
                match lower {
                    Bound::Unbounded => {}
                    Bound::Inclusive(v) => write!(f, ">={v}")?,
                    Bound::Exclusive(v) => write!(f, ">{v}")?,
                }
                if lower != &Bound::Unbounded && upper != &Bound::Unbounded {
                    f.write_str(" ")?;
                }
                match upper {
                    Bound::Unbounded => {}
                    Bound::Inclusive(v) => write!(f, "<={v}")?,
                    Bound::Exclusive(v) => write!(f, "<{v}")?,
                }
            }
        }
        // Only intersections can differ from what the bounds imply.
        if self.prerelease_cores != self.implied_cores() {
            let list: Vec<String> = self
                .prerelease_cores
                .iter()
                .map(|(a, b, c)| format!("{a}.{b}.{c}"))
                .collect();
            let text = if list.is_empty() { "none".to_string() } else { list.join(", ") };
            write!(f, " [pre: {text}]")?;
        }
        Ok(())
    }
}

/// A parsed constraint. `source` keeps the text as written, `Display` prints the
/// normalized form. Two constraints are equal when their intervals are equal,
/// whatever their spelling.
#[derive(Debug, Clone)]
pub struct Constraint {
    source: String,
    intervals: Vec<Interval>,
}

impl PartialEq for Constraint {
    fn eq(&self, other: &Self) -> bool {
        self.intervals == other.intervals
    }
}

impl Eq for Constraint {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstraintError {
    pub input: String,
    /// Byte offset of the offending token within `input`.
    pub offset: usize,
    pub message: String,
}

impl fmt::Display for ConstraintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid constraint `{}` at column {}: {}",
            self.input,
            self.offset + 1,
            self.message
        )
    }
}

impl std::error::Error for ConstraintError {}

impl Constraint {
    pub fn any() -> Self {
        Constraint { source: "*".to_string(), intervals: vec![Interval::everything()] }
    }

    pub fn exact(version: &Version) -> Self {
        let v = version.without_build();
        let prerelease_cores = if v.is_prerelease() { vec![v.core()] } else { Vec::new() };
        Constraint {
            source: format!("={v}"),
            intervals: vec![Interval {
                lower: Bound::Inclusive(v.clone()),
                upper: Bound::Inclusive(v),
                prerelease_cores,
            }],
        }
    }

    pub fn parse(input: &str) -> Result<Self, ConstraintError> {
        if input.trim().is_empty() {
            return Ok(Constraint { source: input.to_string(), ..Constraint::any() });
        }
        let mut intervals = Vec::new();
        let mut offset = 0;
        for alternative in input.split("||") {
            let parsed = parse_group(alternative).map_err(|(at, message)| ConstraintError {
                input: input.to_string(),
                offset: offset + at,
                message,
            })?;
            intervals.push(parsed);
            offset += alternative.len() + 2;
        }
        Ok(Constraint { source: input.to_string(), intervals: canonicalize(intervals) })
    }

    /// The text this constraint was parsed from.
    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn intervals(&self) -> &[Interval] {
        &self.intervals
    }

    /// True when no version at all can match.
    pub fn is_empty(&self) -> bool {
        self.intervals.is_empty()
    }

    /// Tests a version under the default prerelease rule.
    pub fn matches(&self, version: &Version) -> bool {
        self.matches_with(version, false)
    }

    /// Tests a version, with `include_prerelease` lifting the core restriction.
    pub fn matches_with(&self, version: &Version, include_prerelease: bool) -> bool {
        self.intervals.iter().any(|i| i.admits(version, include_prerelease))
    }

    /// The constraint matching exactly what both inputs match.
    pub fn intersect(&self, other: &Constraint) -> Constraint {
        let mut pieces = Vec::new();
        for a in &self.intervals {
            for b in &other.intervals {
                pieces.push(a.intersect(b));
            }
        }
        let intervals = canonicalize(pieces);
        let mut combined = Constraint { source: String::new(), intervals };
        combined.source = combined.to_string();
        combined
    }
}

impl fmt::Display for Constraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.intervals.is_empty() {
            return f.write_str("<none>");
        }
        for (i, interval) in self.intervals.iter().enumerate() {
            if i > 0 {
                f.write_str(" || ")?;
            }
            write!(f, "{interval}")?;
        }
        Ok(())
    }
}

/// Sorts intervals and merges neighbours with identical prerelease permissions.
fn canonicalize(raw: Vec<Interval>) -> Vec<Interval> {
    let mut items: Vec<Interval> = raw.into_iter().filter_map(Interval::normalized).collect();
    items.sort_by(|a, b| {
        cmp_lower(&a.lower, &b.lower)
            .then_with(|| cmp_upper(&a.upper, &b.upper))
            .then_with(|| a.prerelease_cores.cmp(&b.prerelease_cores))
    });
    let mut merged: Vec<Interval> = Vec::with_capacity(items.len());
    for item in items {
        match merged.last_mut() {
            Some(last)
                if last.prerelease_cores == item.prerelease_cores
                    && touches(&last.upper, &item.lower) =>
            {
                if cmp_upper(&item.upper, &last.upper) == Ordering::Greater {
                    last.upper = item.upper;
                }
            }
            _ => merged.push(item),
        }
    }
    merged
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Plain,
    Caret,
    Tilde,
    Gt,
    Ge,
    Lt,
    Le,
    Eq,
}

struct Token<'a> {
    offset: usize,
    op: Op,
    body: &'a str,
}

/// A version as written, possibly partial.
enum Spec {
    Any,
    Major(u64),
    Minor(u64, u64),
    Full(Version),
}

const OPERATORS: [(&str, Op); 8] = [
    (">=", Op::Ge),
    ("<=", Op::Le),
    ("~>", Op::Tilde),
    (">", Op::Gt),
    ("<", Op::Lt),
    ("=", Op::Eq),
    ("^", Op::Caret),
    ("~", Op::Tilde),
];

fn is_separator(c: char) -> bool {
    c.is_whitespace() || c == ','
}

/// Splits one AND group into operator and version tokens. A token ends at a
/// separator or where the next operator starts, so `>=1.0<2.0` also works.
fn tokenize(text: &str) -> Result<Vec<Token<'_>>, (usize, String)> {
    let mut tokens = Vec::new();
    let mut pos = 0;
    loop {
        pos += text[pos..].len() - text[pos..].trim_start_matches(is_separator).len();
        if pos == text.len() {
            return Ok(tokens);
        }
        let offset = pos;
        let mut op = Op::Plain;
        if let Some((symbol, found)) =
            OPERATORS.iter().find(|(symbol, _)| text[pos..].starts_with(symbol))
        {
            op = *found;
            pos += symbol.len();
            pos += text[pos..].len() - text[pos..].trim_start().len();
        }
        let rest = &text[pos..];
        let end = rest
            .find(|c| is_separator(c) || matches!(c, '<' | '>' | '=' | '^' | '~'))
            .unwrap_or(rest.len());
        if end == 0 {
            return Err((offset, "operator without a version".to_string()));
        }
        tokens.push(Token { offset, op, body: &rest[..end] });
        pos += end;
    }
}

fn parse_spec(body: &str) -> Result<Spec, String> {
    let body = body.strip_prefix(['v', 'V']).unwrap_or(body);
    let core_end = body.find(['-', '+']).unwrap_or(body.len());
    let (core, suffix) = body.split_at(core_end);

    let mut numbers: Vec<Option<u64>> = Vec::new();
    for part in core.split('.') {
        numbers.push(match part {
            "*" | "x" | "X" => None,
            _ => Some(parse_numeric(part)?),
        });
    }
    if numbers.len() > 3 {
        return Err(format!("`{body}` has more than three components"));
    }
    let specific = numbers.iter().take_while(|n| n.is_some()).count();
    if numbers[specific..].iter().any(Option::is_some) {
        return Err(format!("`{body}` has a number after a wildcard"));
    }
    if specific < 3 && !suffix.is_empty() {
        return Err(format!("`{body}` needs a full version to carry a prerelease or build"));
    }
    Ok(match (specific, numbers.first(), numbers.get(1)) {
        (3, ..) => Spec::Full(Version::parse(body).map_err(|e| e.reason)?.without_build()),
        (2, Some(Some(major)), Some(Some(minor))) => Spec::Minor(*major, *minor),
        (1, Some(Some(major)), _) => Spec::Major(*major),
        _ => Spec::Any,
    })
}

fn increment(n: u64) -> Result<u64, String> {
    n.checked_add(1).ok_or_else(|| format!("{n} is too large to compute an upper bound"))
}

impl Spec {
    /// The first release this spec covers.
    fn floor(&self) -> Version {
        match self {
            Spec::Any => Version::new(0, 0, 0),
            Spec::Major(m) => Version::new(*m, 0, 0),
            Spec::Minor(m, n) => Version::new(*m, *n, 0),
            Spec::Full(v) => v.clone(),
        }
    }

    /// The bound just past the last version this spec covers.
    fn ceiling(&self) -> Result<Bound, String> {
        Ok(match self {
            Spec::Any => Bound::Unbounded,
            Spec::Major(m) => Bound::Exclusive(Version::lowest_prerelease(increment(*m)?, 0, 0)),
            Spec::Minor(m, n) => Bound::Exclusive(Version::lowest_prerelease(*m, increment(*n)?, 0)),
            Spec::Full(v) => Bound::Inclusive(v.clone()),
        })
    }

    /// The first release after this partial spec, for `>1` and `>1.2`.
    fn after(&self) -> Result<Version, String> {
        Ok(match self {
            Spec::Major(m) => Version::new(increment(*m)?, 0, 0),
            Spec::Minor(m, n) => Version::new(*m, increment(*n)?, 0),
            Spec::Any | Spec::Full(_) => self.floor(),
        })
    }

    /// The core whose prereleases this spec admits, when it names one.
    fn prerelease_core(&self) -> Vec<Core> {
        match self {
            Spec::Full(v) if v.is_prerelease() => vec![v.core()],
            _ => Vec::new(),
        }
    }

    fn caret_ceiling(&self) -> Result<Bound, String> {
        let (major, minor, patch) = match self {
            Spec::Any => return Ok(Bound::Unbounded),
            Spec::Major(m) => (increment(*m)?, 0, 0),
            Spec::Minor(0, n) => (0, increment(*n)?, 0),
            Spec::Minor(m, _) => (increment(*m)?, 0, 0),
            Spec::Full(v) if v.major > 0 => (increment(v.major)?, 0, 0),
            Spec::Full(v) if v.minor > 0 => (0, increment(v.minor)?, 0),
            Spec::Full(v) => (0, 0, increment(v.patch)?),
        };
        Ok(Bound::Exclusive(Version::lowest_prerelease(major, minor, patch)))
    }

    fn tilde_ceiling(&self) -> Result<Bound, String> {
        match self {
            Spec::Full(v) => Ok(Bound::Exclusive(Version::lowest_prerelease(
                v.major,
                increment(v.minor)?,
                0,
            ))),
            partial => partial.ceiling(),
        }
    }
}

/// Compiles one comparator to its interval.
fn comparator(op: Op, spec: &Spec) -> Result<Interval, String> {
    let any = matches!(spec, Spec::Any);
    let full = matches!(spec, Spec::Full(_));
    let from_floor = if any { Bound::Unbounded } else { Bound::Inclusive(spec.floor()) };
    let (lower, upper) = match op {
        Op::Plain | Op::Eq => (from_floor, spec.ceiling()?),
        Op::Caret => (from_floor, spec.caret_ceiling()?),
        Op::Tilde => (from_floor, spec.tilde_ceiling()?),
        Op::Ge => (from_floor, Bound::Unbounded),
        Op::Le => (Bound::Unbounded, spec.ceiling()?),
        Op::Gt | Op::Lt if any => return Ok(Interval::nothing()),
        Op::Gt if full => (Bound::Exclusive(spec.floor()), Bound::Unbounded),
        Op::Gt => (Bound::Inclusive(spec.after()?), Bound::Unbounded),
        Op::Lt if full => (Bound::Unbounded, Bound::Exclusive(spec.floor())),
        Op::Lt => {
            let floor = spec.floor();
            let below = Version::lowest_prerelease(floor.major, floor.minor, floor.patch);
            (Bound::Unbounded, Bound::Exclusive(below))
        }
    };
    Ok(Interval { lower, upper, prerelease_cores: spec.prerelease_core() })
}

/// Parses one AND group, between `||` separators.
fn parse_group(text: &str) -> Result<Interval, (usize, String)> {
    let tokens = tokenize(text)?;
    if tokens.is_empty() {
        return Err((text.len(), "empty alternative".to_string()));
    }
    let spec_at = |t: &Token<'_>| parse_spec(t.body).map_err(|m| (t.offset, m));
    let compile = |t: &Token<'_>, op: Op| {
        comparator(op, &spec_at(t)?).map_err(|m| (t.offset, m))
    };

    if let Some(dash) = tokens.iter().find(|t| t.op == Op::Plain && t.body == "-") {
        let [from, _, to] = tokens.as_slice() else {
            return Err((dash.offset, "a hyphen range must stand alone in its group".to_string()));
        };
        if from.body == "-" || to.body == "-" || from.op != Op::Plain || to.op != Op::Plain {
            return Err((dash.offset, "a hyphen range joins two plain versions".to_string()));
        }
        return Ok(compile(from, Op::Ge)?.and(&compile(to, Op::Le)?));
    }

    let mut interval = Interval::everything();
    for token in &tokens {
        interval = interval.and(&compile(token, token.op)?);
    }
    Ok(interval)
}
