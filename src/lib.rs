//! SemVer versions and constraints, and a backtracking dependency resolver with
//! readable conflict reports. No external crates; `json` is a small in-tree parser.
//!
//! ```
//! use semver_resolver::{resolve, Manifest, Options, Registry, Version};
//!
//! let registry = Registry::from_json(r#"{"packages": {
//!     "alpha": {"1.0.0": {"dependencies": {"beta": "^2"}}},
//!     "beta": {"2.0.0": {}, "2.4.1": {}, "3.0.0": {}}
//! }}"#).unwrap();
//! let manifest = Manifest::from_json(r#"{"name": "demo", "dependencies": {"alpha": "*"}}"#).unwrap();
//!
//! let solved = resolve(&registry, &manifest, &Options::default()).unwrap();
//! assert_eq!(solved.resolution.packages["beta"].version, Version::new(2, 4, 1));
//! ```

pub mod cli;
pub mod constraint;
pub mod explain;
pub mod graph;
pub mod json;
pub mod lockfile;
pub mod model;
pub mod outdated;
pub mod registry;
pub mod resolver;
pub mod version;

pub use constraint::Constraint;
pub use json::Json;
pub use model::{DataError, Dependency, Manifest, Resolution, ResolvedPackage};
pub use registry::{Registry, Release};
pub use resolver::{resolve, Conflict, Options, Origin, ResolveError, Solved};
pub use version::{Bump, Version};
