//! Views over a resolved package set: the dependency tree, install order, and
//! reverse lookups. Dependency cycles are legal here and are handled throughout.

use crate::constraint::Constraint;
use crate::model::{Dependency, Resolution};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// One step of an install plan. Members of a cycle must be installed together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallStep {
    Single(String),
    Cycle(Vec<String>),
}

/// Renders the dependency tree from the root. A package repeats its subtree only
/// once, later occurrences carry `(*)`, unless `full` asks for every expansion.
/// A back-edge is marked `(cycle)` either way, and is never followed.
pub fn render_tree(resolution: &Resolution, full: bool) -> String {
    let mut out = resolution.root_name.clone();
    if let Some(version) = &resolution.root_version {
        out.push_str(&format!(" {version}"));
    }
    out.push('\n');
    let mut walk = TreeWalk { resolution, full, seen: BTreeSet::new(), path: Vec::new(), out };
    walk.children(&resolution.root_dependencies, "");
    walk.out
}

struct TreeWalk<'a> {
    resolution: &'a Resolution,
    full: bool,
    seen: BTreeSet<&'a str>,
    path: Vec<&'a str>,
    out: String,
}

impl<'a> TreeWalk<'a> {
    fn children(&mut self, dependencies: &'a [Dependency], prefix: &str) {
        for (i, dependency) in dependencies.iter().enumerate() {
            let last = i + 1 == dependencies.len();
            let name = dependency.name.as_str();
            let branch = if last { "└── " } else { "├── " };
            let node = self.resolution.packages.get(name);

            let label = node.map_or("(missing)".to_string(), |n| n.version.to_string());
            let wanted = dependency.constraint.source();
            self.out.push_str(&format!("{prefix}{branch}{name} {label} [{wanted}]"));

            let Some(node) = node else {
                self.out.push('\n');
                continue;
            };
            if self.path.contains(&name) {
                self.out.push_str(" (cycle)\n");
                continue;
            }
            let repeated = !self.full && !self.seen.insert(name) && !node.dependencies.is_empty();
            if repeated {
                self.out.push_str(" (*)\n");
                continue;
            }
            self.out.push('\n');
            self.path.push(name);
            let child_prefix = format!("{prefix}{}", if last { "    " } else { "│   " });
            self.children(&node.dependencies, &child_prefix);
            self.path.pop();
        }
    }
}

/// Orders packages so dependencies come before their dependents. Strongly
/// connected groups become one `Cycle` step. Dependencies missing from the set
/// are ignored. The order is deterministic for a given input.
pub fn install_order(resolution: &Resolution) -> Vec<InstallStep> {
    let mut tarjan = Tarjan {
        resolution,
        next_index: 0,
        index: BTreeMap::new(),
        low: BTreeMap::new(),
        stack: Vec::new(),
        on_stack: BTreeSet::new(),
        steps: Vec::new(),
    };
    for name in resolution.packages.keys() {
        if !tarjan.index.contains_key(name.as_str()) {
            tarjan.visit(name);
        }
    }
    tarjan.steps
}

/// Tarjan's algorithm emits components sinks first, which is already install order.
struct Tarjan<'a> {
    resolution: &'a Resolution,
    next_index: usize,
    index: BTreeMap<&'a str, usize>,
    low: BTreeMap<&'a str, usize>,
    stack: Vec<&'a str>,
    on_stack: BTreeSet<&'a str>,
    steps: Vec<InstallStep>,
}

impl<'a> Tarjan<'a> {
    fn visit(&mut self, name: &'a str) {
        let order = self.next_index;
        self.next_index += 1;
        self.index.insert(name, order);
        self.low.insert(name, order);
        self.stack.push(name);
        self.on_stack.insert(name);

        let mut lowest = order;
        let resolution = self.resolution;
        for dependency in &resolution.packages[name].dependencies {
            let next = dependency.name.as_str();
            if !resolution.packages.contains_key(next) {
                continue;
            }
            if let Some(&seen) = self.index.get(next) {
                if self.on_stack.contains(next) {
                    lowest = lowest.min(seen);
                }
            } else {
                self.visit(next);
                lowest = lowest.min(self.low[next]);
            }
        }
        self.low.insert(name, lowest);

        if lowest == order {
            let mut members = Vec::new();
            while let Some(member) = self.stack.pop() {
                self.on_stack.remove(member);
                members.push(member.to_string());
                if member == name {
                    break;
                }
            }
            members.sort();
            self.steps.push(if members.len() == 1 {
                InstallStep::Single(members.remove(0))
            } else {
                InstallStep::Cycle(members)
            });
        }
    }
}

/// Everything that requires `name`, as (description, constraint); the root first.
pub fn dependents<'a>(resolution: &'a Resolution, name: &str) -> Vec<(String, &'a Constraint)> {
    let mut found = Vec::new();
    for dependency in resolution.root_dependencies.iter().filter(|d| d.name == name) {
        found.push((format!("{} (root)", resolution.root_name), &dependency.constraint));
    }
    for (who, package) in &resolution.packages {
        for dependency in package.dependencies.iter().filter(|d| d.name == name) {
            found.push((format!("{who} {}", package.version), &dependency.constraint));
        }
    }
    found
}

/// One shortest chain of names from the root to `name`, or `None` if unreachable.
pub fn path_from_root(resolution: &Resolution, name: &str) -> Option<Vec<String>> {
    let mut parent: BTreeMap<&str, &str> = BTreeMap::new();
    let mut queue: VecDeque<&str> = VecDeque::new();
    for dependency in &resolution.root_dependencies {
        if !parent.contains_key(dependency.name.as_str()) {
            parent.insert(&dependency.name, "");
            queue.push_back(&dependency.name);
        }
    }
    while let Some(current) = queue.pop_front() {
        if current == name {
            let mut chain = vec![current.to_string()];
            let mut step = current;
            while let Some(&up) = parent.get(step).filter(|up| !up.is_empty()) {
                chain.push(up.to_string());
                step = up;
            }
            chain.push(resolution.root_name.clone());
            chain.reverse();
            return Some(chain);
        }
        let Some(package) = resolution.packages.get(current) else {
            continue;
        };
        for dependency in &package.dependencies {
            if !parent.contains_key(dependency.name.as_str()) {
                parent.insert(&dependency.name, current);
                queue.push_back(&dependency.name);
            }
        }
    }
    None
}
