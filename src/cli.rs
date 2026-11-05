//! The `semres` command line. Exit codes: 0 success, 1 the operation found a
//! problem (no solution, lockfile mismatch), 2 bad usage or unreadable input.

use crate::explain::explain_package;
use crate::graph::{self, InstallStep};
use crate::lockfile;
use crate::model::{Manifest, Resolution};
use crate::outdated;
use crate::registry::Registry;
use crate::resolver::{resolve, Options, Solved};
use crate::version::Version;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::io::Write;
use std::path::Path;

const USAGE: &str = "\
semres: resolve SemVer dependencies against a JSON registry

usage: semres <command> [options]

commands:
  resolve               resolve a manifest and print or lock the result
  tree                  print the dependency tree
  sort                  print the install order, dependencies first
  check                 verify a lockfile against the manifest and registry
  explain <package>     say why a package is at its version, or why nothing fits
  outdated              list locked packages with newer versions

input options:
  -r, --registry <file>  registry JSON
  -m, --manifest <file>  manifest JSON
  -l, --lock <file>      lockfile: preferred by resolve, read by the other commands

solver options (resolve, tree, sort, explain):
  --prefer-lowest        try lowest versions first
  --pin <name=version>   force a version; repeatable
  --pre                  let prereleases match for every package
  --pre-for <name>       let prereleases match for one package; repeatable
  --max-steps <n>        give up after trying this many versions

command options:
  resolve: --write       write the result to the --lock file
  tree:    --full        expand repeated subtrees
";

/// A command's stdout and exit code; failures that print no report use `CliError`.
struct Output {
    text: String,
    code: i32,
}

impl Output {
    fn ok(text: String) -> Self {
        Output { text, code: 0 }
    }
}

enum CliError {
    /// Bad arguments or unreadable input: exit 2.
    Usage(String),
    /// The operation ran and failed: exit 1.
    Failed(String),
}

fn usage_err(message: impl std::fmt::Display) -> CliError {
    CliError::Usage(message.to_string())
}

/// Runs the CLI against the given writers and returns the process exit code.
pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let result = dispatch(args);
    // A closed pipe on stdout or stderr should not turn into a panic.
    match result {
        Ok(output) => {
            let _ = out.write_all(output.text.as_bytes());
            output.code
        }
        Err(CliError::Failed(message)) => {
            let _ = write!(err, "{message}");
            if !message.ends_with('\n') {
                let _ = writeln!(err);
            }
            1
        }
        Err(CliError::Usage(message)) => {
            let _ = writeln!(err, "error: {message}");
            2
        }
    }
}

fn dispatch(args: &[String]) -> Result<Output, CliError> {
    let Some((command, rest)) = args.split_first() else {
        return Err(usage_err(format!("no command given\n\n{USAGE}")));
    };
    match command.as_str() {
        "resolve" => cmd_resolve(rest),
        "tree" => cmd_tree(rest),
        "sort" => cmd_sort(rest),
        "check" => cmd_check(rest),
        "explain" => cmd_explain(rest),
        "outdated" => cmd_outdated(rest),
        "help" | "-h" | "--help" => Ok(Output::ok(USAGE.to_string())),
        other => Err(usage_err(format!("unknown command `{other}`, try `semres help`"))),
    }
}

/// Parsed arguments for one command.
#[derive(Default)]
struct Args {
    positional: Vec<String>,
    values: BTreeMap<String, Vec<String>>,
    flags: BTreeSet<String>,
}

const SOLVER_FLAGS: [&str; 2] = ["--prefer-lowest", "--pre"];
const INPUT_VALUES: [&str; 3] = ["--registry", "--manifest", "--lock"];
const ALL_VALUES: [&str; 6] =
    ["--registry", "--manifest", "--lock", "--pin", "--pre-for", "--max-steps"];

impl Args {
    fn parse(raw: &[String], value_opts: &[&str], flag_opts: &[&str]) -> Result<Args, CliError> {
        let mut args = Args::default();
        let mut iter = raw.iter();
        while let Some(arg) = iter.next() {
            let (name, inline) = match arg.split_once('=') {
                Some((name, value)) if name.starts_with("--") => (name, Some(value.to_string())),
                _ => (arg.as_str(), None),
            };
            let name = match name {
                "-r" => "--registry",
                "-m" => "--manifest",
                "-l" => "--lock",
                other => other,
            };
            if value_opts.contains(&name) {
                let value = match inline.or_else(|| iter.next().cloned()) {
                    Some(value) => value,
                    None => return Err(usage_err(format!("{name} needs a value"))),
                };
                args.values.entry(name.to_string()).or_default().push(value);
            } else if flag_opts.contains(&name) && inline.is_none() {
                args.flags.insert(name.to_string());
            } else if name.starts_with('-') && name.len() > 1 {
                return Err(usage_err(format!("unknown option `{arg}`")));
            } else {
                args.positional.push(arg.clone());
            }
        }
        Ok(args)
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.values.get(name).and_then(|list| list.last()).map(String::as_str)
    }

    fn all(&self, name: &str) -> &[String] {
        self.values.get(name).map_or(&[], Vec::as_slice)
    }

    fn flag(&self, name: &str) -> bool {
        self.flags.contains(name)
    }

    fn required(&self, name: &str) -> Result<&str, CliError> {
        self.value(name).ok_or_else(|| usage_err(format!("{name} is required")))
    }

    fn no_positional(&self) -> Result<(), CliError> {
        match self.positional.first() {
            Some(extra) => Err(usage_err(format!("unexpected argument `{extra}`"))),
            None => Ok(()),
        }
    }
}

fn read_file(path: &str) -> Result<String, CliError> {
    fs::read_to_string(path).map_err(|e| usage_err(format!("cannot read {path}: {e}")))
}

fn load_registry(args: &Args) -> Result<Registry, CliError> {
    let path = args.required("--registry")?;
    Registry::from_json(&read_file(path)?).map_err(|e| usage_err(format!("{path}: {e}")))
}

fn load_manifest(args: &Args) -> Result<Manifest, CliError> {
    let path = args.required("--manifest")?;
    Manifest::from_json(&read_file(path)?).map_err(|e| usage_err(format!("{path}: {e}")))
}

fn load_lock(path: &str) -> Result<Resolution, CliError> {
    lockfile::read(&read_file(path)?).map_err(|e| usage_err(format!("{path}: {e}")))
}

fn solver_options(args: &Args) -> Result<Options, CliError> {
    let mut options = Options {
        prefer_lowest: args.flag("--prefer-lowest"),
        prerelease_all: args.flag("--pre"),
        prerelease_packages: args.all("--pre-for").iter().cloned().collect(),
        ..Options::default()
    };
    for pin in args.all("--pin") {
        let (name, version) = pin
            .split_once('=')
            .ok_or_else(|| usage_err(format!("--pin expects name=version, got `{pin}`")))?;
        let version = Version::parse(version).map_err(usage_err)?;
        options.pins.insert(name.to_string(), version);
    }
    if let Some(limit) = args.value("--max-steps") {
        options.max_steps = limit
            .parse()
            .map_err(|_| usage_err(format!("--max-steps expects a number, got `{limit}`")))?;
    }
    Ok(options)
}

fn solve(args: &Args, options: &Options) -> Result<Solved, CliError> {
    let registry = load_registry(args)?;
    let manifest = load_manifest(args)?;
    resolve(&registry, &manifest, options).map_err(|e| CliError::Failed(e.to_string()))
}

/// The graph for read-only views: the lockfile if given, otherwise a fresh solve.
fn load_graph(args: &Args) -> Result<Resolution, CliError> {
    match args.value("--lock") {
        Some(path) => load_lock(path),
        None => Ok(solve(args, &solver_options(args)?)?.resolution),
    }
}

fn cmd_resolve(raw: &[String]) -> Result<Output, CliError> {
    let flags: Vec<&str> = SOLVER_FLAGS.iter().chain(&["--write"]).copied().collect();
    let args = Args::parse(raw, &ALL_VALUES, &flags)?;
    args.no_positional()?;
    if args.flag("--write") && args.value("--lock").is_none() {
        return Err(usage_err("--write needs --lock <file>"));
    }

    let mut options = solver_options(&args)?;
    if let Some(path) = args.value("--lock").filter(|path| Path::new(path).exists()) {
        options.preferred = lockfile::preferred_versions(&load_lock(path)?);
    }

    let solved = solve(&args, &options)?;
    if args.flag("--write") {
        // Checked above: --write implies --lock.
        let path = args.required("--lock")?;
        let text = lockfile::write(&solved.resolution);
        fs::write(path, text).map_err(|e| usage_err(format!("cannot write {path}: {e}")))?;
    }

    let packages = &solved.resolution.packages;
    let mut out = format!(
        "resolved {} packages in {} steps, {} backtracks\n",
        packages.len(),
        solved.steps,
        solved.backtracks
    );
    for (name, package) in packages {
        let _ = writeln!(out, "  {name} {}", package.version);
    }
    Ok(Output::ok(out))
}

fn cmd_tree(raw: &[String]) -> Result<Output, CliError> {
    let flags: Vec<&str> = SOLVER_FLAGS.iter().chain(&["--full"]).copied().collect();
    let args = Args::parse(raw, &ALL_VALUES, &flags)?;
    args.no_positional()?;
    Ok(Output::ok(graph::render_tree(&load_graph(&args)?, args.flag("--full"))))
}

fn cmd_sort(raw: &[String]) -> Result<Output, CliError> {
    let args = Args::parse(raw, &ALL_VALUES, &SOLVER_FLAGS)?;
    args.no_positional()?;
    let resolution = load_graph(&args)?;
    let label = |name: &String| format!("{name} {}", resolution.packages[name].version);
    let mut out = String::new();
    for step in graph::install_order(&resolution) {
        match step {
            InstallStep::Single(name) => {
                let _ = writeln!(out, "{}", label(&name));
            }
            InstallStep::Cycle(names) => {
                let members: Vec<String> = names.iter().map(label).collect();
                let _ = writeln!(out, "cycle: {}", members.join(", "));
            }
        }
    }
    Ok(Output::ok(out))
}

fn cmd_check(raw: &[String]) -> Result<Output, CliError> {
    let args = Args::parse(raw, &INPUT_VALUES, &["--pre"])?;
    args.no_positional()?;
    let registry = load_registry(&args)?;
    let manifest = load_manifest(&args)?;
    let locked = load_lock(args.required("--lock")?)?;
    let options = Options { prerelease_all: args.flag("--pre"), ..Options::default() };

    let problems = lockfile::verify(&locked, &manifest, &registry, &options);
    if problems.is_empty() {
        return Ok(Output::ok(format!("lockfile ok: {} packages\n", locked.packages.len())));
    }
    let mut text = format!("lockfile has {} problem(s):\n", problems.len());
    for problem in &problems {
        let _ = writeln!(text, "  {problem}");
    }
    Ok(Output { text, code: 1 })
}

fn cmd_explain(raw: &[String]) -> Result<Output, CliError> {
    let args = Args::parse(raw, &ALL_VALUES, &SOLVER_FLAGS)?;
    let [package] = args.positional.as_slice() else {
        return Err(usage_err("explain takes exactly one package name"));
    };
    let registry = load_registry(&args)?;
    let manifest = load_manifest(&args)?;
    let resolution = load_graph(&args)?;
    let options = solver_options(&args)?.with_manifest_pins(&manifest);
    match explain_package(&resolution, &registry, &options, package) {
        Some(explanation) => Ok(Output::ok(explanation.to_string())),
        None => Err(CliError::Failed(format!("{package} is not part of the resolution"))),
    }
}

fn cmd_outdated(raw: &[String]) -> Result<Output, CliError> {
    let args = Args::parse(raw, &["--registry", "--lock"], &["--pre"])?;
    args.no_positional()?;
    let registry = load_registry(&args)?;
    let locked = load_lock(args.required("--lock")?)?;
    let options = Options { prerelease_all: args.flag("--pre"), ..Options::default() };
    Ok(Output::ok(outdated::report(&locked, &registry, &options).to_table()))
}
