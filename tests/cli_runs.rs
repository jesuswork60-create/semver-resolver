//! Runs the real `semres` binary against the example data and scratch files.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn semres(args: &[&str]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_semres"))
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("binary runs");
    Run {
        code: output.status.code().expect("exited normally"),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

const REGISTRY: &str = "examples/registry.json";
const MANIFEST: &str = "examples/manifest.json";
const CONFLICT: &str = "examples/conflict.json";

/// A fresh directory under Cargo's per-target scratch area.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn resolve_prints_the_selection() {
    let run = semres(&["resolve", "-r", REGISTRY, "-m", MANIFEST]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stdout.starts_with("resolved 5 packages"), "{}", run.stdout);
    for line in ["bricks 1.4.2", "lantern 1.0.0", "marmot 1.0.0", "nest 1.1.0", "perch 1.0.0"] {
        assert!(run.stdout.contains(line), "missing {line} in {}", run.stdout);
    }
}

#[test]
fn resolve_solver_flags_change_the_outcome() {
    let lowest = semres(&["resolve", "-r", REGISTRY, "-m", MANIFEST, "--prefer-lowest"]);
    assert!(lowest.stdout.contains("marmot 0.9.0"), "{}", lowest.stdout);
    assert!(lowest.stdout.contains("bricks 1.4.2") || lowest.stdout.contains("bricks 1.2"), "{}", lowest.stdout);

    let pinned = semres(&["resolve", "-r", REGISTRY, "-m", MANIFEST, "--pin", "marmot=0.9.0"]);
    assert!(pinned.stdout.contains("marmot 0.9.0"), "{}", pinned.stdout);
    assert!(!pinned.stdout.contains("nest"), "{}", pinned.stdout);

    let inline = semres(&["resolve", "-r", REGISTRY, "-m", MANIFEST, "--pin=marmot=0.9.0"]);
    assert_eq!(inline.stdout, pinned.stdout);
}

#[test]
fn prerelease_flags_reach_the_solver() {
    let dir = scratch("pre");
    let manifest = dir.join("m.json");
    fs::write(&manifest, r#"{"name": "x", "dependencies": {"bricks": ">=1.4 <2.0.0"}}"#).unwrap();
    let path = manifest.to_str().unwrap();

    let plain = semres(&["resolve", "-r", REGISTRY, "-m", path]);
    assert!(plain.stdout.contains("bricks 1.4.2"), "{}", plain.stdout);
    let all = semres(&["resolve", "-r", REGISTRY, "-m", path, "--pre"]);
    assert!(all.stdout.contains("bricks 2.0.0-rc.1"), "{}", all.stdout);
    let one = semres(&["resolve", "-r", REGISTRY, "-m", path, "--pre-for", "bricks"]);
    assert!(one.stdout.contains("bricks 2.0.0-rc.1"), "{}", one.stdout);
    let other = semres(&["resolve", "-r", REGISTRY, "-m", path, "--pre-for", "wick"]);
    assert!(other.stdout.contains("bricks 1.4.2"), "{}", other.stdout);
}

#[test]
fn conflict_goes_to_stderr_with_exit_one() {
    let run = semres(&["resolve", "-r", REGISTRY, "-m", CONFLICT]);
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
    assert!(run.stderr.contains("cannot resolve"), "{}", run.stderr);
    assert!(run.stderr.contains("bricks"), "{}", run.stderr);
    assert!(run.stderr.contains("from shed (root)"), "{}", run.stderr);
}

#[test]
fn step_budget_flag_is_honoured() {
    let run = semres(&["resolve", "-r", REGISTRY, "-m", MANIFEST, "--max-steps", "1"]);
    assert_eq!(run.code, 1);
    assert!(run.stderr.contains("gave up after 1 steps"), "{}", run.stderr);
}

#[test]
fn lock_write_honour_and_check() {
    let dir = scratch("lock");
    let lock = dir.join("cabin.lock");
    let lock_path = lock.to_str().unwrap();

    let first = semres(&["resolve", "-r", REGISTRY, "-m", MANIFEST, "-l", lock_path, "--write"]);
    assert_eq!(first.code, 0, "{}", first.stderr);
    let written = fs::read_to_string(&lock).unwrap();
    assert!(written.contains("\"marmot\""), "{written}");

    let check = semres(&["check", "-r", REGISTRY, "-m", MANIFEST, "-l", lock_path]);
    assert_eq!((check.code, check.stdout.as_str()), (0, "lockfile ok: 5 packages\n"));

    // A hand-edited lock on an older acceptable version is respected...
    let older = written.replacen("\"version\": \"1.1.0\"", "\"version\": \"1.0.0\"", 1);
    assert_ne!(older, written);
    fs::write(&lock, &older).unwrap();
    let kept = semres(&["resolve", "-r", REGISTRY, "-m", MANIFEST, "-l", lock_path]);
    assert_eq!(kept.code, 0, "{}", kept.stderr);
    assert!(kept.stdout.contains("nest 1.0.0"), "{}", kept.stdout);

    // ...and one the solver cannot keep is replaced in the answer, not on disk.
    let broken = written.replacen("\"version\": \"1.4.2\"", "\"version\": \"2.0.0\"", 1);
    assert_ne!(broken, written);
    fs::write(&lock, &broken).unwrap();
    let moved = semres(&["resolve", "-r", REGISTRY, "-m", MANIFEST, "-l", lock_path]);
    assert_eq!(moved.code, 0, "{}", moved.stderr);
    assert!(moved.stdout.contains("bricks 1.4.2"), "{}", moved.stdout);
    assert_eq!(fs::read_to_string(&lock).unwrap(), broken, "only --write touches the lock");

    let bad = semres(&["check", "-r", REGISTRY, "-m", MANIFEST, "-l", lock_path]);
    assert_eq!(bad.code, 1);
    assert!(bad.stdout.contains("bricks 2.0.0 does not satisfy"), "{}", bad.stdout);

    let missing = dir.join("absent.lock");
    let absent = semres(&["resolve", "-r", REGISTRY, "-m", MANIFEST, "-l", missing.to_str().unwrap()]);
    assert_eq!(absent.code, 0, "a lock that does not exist yet is fine: {}", absent.stderr);
    assert!(!missing.exists());
}

#[test]
fn tree_and_sort_work_from_a_solve_or_a_lock_alone() {
    let solved = semres(&["tree", "-r", REGISTRY, "-m", MANIFEST]);
    assert_eq!(solved.code, 0, "{}", solved.stderr);
    assert!(solved.stdout.starts_with("cabin 0.1.0\n"), "{}", solved.stdout);
    assert!(solved.stdout.contains("(cycle)"), "{}", solved.stdout);

    let dir = scratch("views");
    let lock = dir.join("lock.json");
    let lock_path = lock.to_str().unwrap();
    semres(&["resolve", "-r", REGISTRY, "-m", MANIFEST, "-l", lock_path, "--write"]);

    let from_lock = semres(&["tree", "-l", lock_path]);
    assert_eq!(from_lock.stdout, solved.stdout);
    assert_eq!(semres(&["tree", "-l", lock_path, "--full"]).code, 0);

    let sorted = semres(&["sort", "-l", lock_path]);
    assert_eq!(sorted.code, 0, "{}", sorted.stderr);
    let lines: Vec<&str> = sorted.stdout.lines().collect();
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert!(lines.contains(&"cycle: nest 1.1.0, perch 1.0.0"), "{lines:?}");
    let at = |needle: &str| lines.iter().position(|l| l.contains(needle)).unwrap();
    assert!(at("bricks") < at("marmot"));
    assert!(at("nest") < at("marmot"));
    assert_eq!(sorted.stdout, semres(&["sort", "-r", REGISTRY, "-m", MANIFEST]).stdout);
}

#[test]
fn explain_answers_for_present_absent_and_unsolvable() {
    let run = semres(&["explain", "bricks", "-r", REGISTRY, "-m", MANIFEST]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stdout.contains("bricks 1.4.2 is selected"), "{}", run.stdout);
    assert!(run.stdout.contains("2.0.0  rejected by ^1.0 from lantern 1.0.0"), "{}", run.stdout);

    let absent = semres(&["explain", "wick", "-r", REGISTRY, "-m", MANIFEST]);
    assert_eq!(absent.code, 1);
    assert!(absent.stderr.contains("wick is not part of the resolution"), "{}", absent.stderr);

    let failed = semres(&["explain", "bricks", "-r", REGISTRY, "-m", CONFLICT]);
    assert_eq!(failed.code, 1);
    assert!(failed.stderr.contains("cannot resolve"), "{}", failed.stderr);
}

#[test]
fn outdated_prints_a_table() {
    let dir = scratch("outdated");
    let lock = dir.join("lock.json");
    let lock_path = lock.to_str().unwrap();
    semres(&["resolve", "-r", REGISTRY, "-m", MANIFEST, "-l", lock_path, "--write"]);

    let table = semres(&["outdated", "-r", REGISTRY, "-l", lock_path]);
    assert_eq!(table.code, 0, "{}", table.stderr);
    assert!(table.stdout.contains("lantern"), "{}", table.stdout);
    assert!(table.stdout.contains("perch (indirect)"), "{}", table.stdout);
}

#[test]
fn usage_errors_exit_two() {
    let table: &[&[&str]] = &[
        &[],
        &["frobnicate"],
        &["resolve"],
        &["resolve", "-r", REGISTRY],
        &["resolve", "-r", "no-such-file.json", "-m", MANIFEST],
        &["resolve", "-r", REGISTRY, "-m", MANIFEST, "--bogus"],
        &["resolve", "-r", REGISTRY, "-m", MANIFEST, "extra"],
        &["resolve", "-r", REGISTRY, "-m", MANIFEST, "--write"],
        &["resolve", "-r", REGISTRY, "-m", MANIFEST, "--pin", "nope"],
        &["resolve", "-r", REGISTRY, "-m", MANIFEST, "--pin", "a=^1"],
        &["resolve", "-r", REGISTRY, "-m", MANIFEST, "--max-steps", "lots"],
        &["resolve", "-r", REGISTRY, "-m", MANIFEST, "--max-steps"],
        &["resolve", "-r", MANIFEST, "-m", MANIFEST],
        &["tree"],
        &["check", "-r", REGISTRY, "-m", MANIFEST],
        &["explain", "-r", REGISTRY, "-m", MANIFEST],
        &["explain", "a", "b", "-r", REGISTRY, "-m", MANIFEST],
        &["outdated", "-r", REGISTRY],
    ];
    for args in table {
        let run = semres(args);
        assert_eq!(run.code, 2, "args {args:?}: stdout {} stderr {}", run.stdout, run.stderr);
        assert!(run.stderr.starts_with("error: "), "args {args:?}: {}", run.stderr);
    }
}

#[test]
fn bad_data_files_name_the_file_and_location() {
    let dir = scratch("baddata");
    let reg = dir.join("reg.json");
    fs::write(&reg, "{\n  \"packages\": oops\n}").unwrap();
    let reg_path = reg.to_str().unwrap();
    let run = semres(&["resolve", "-r", reg_path, "-m", MANIFEST]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains(reg_path), "{}", run.stderr);
    assert!(run.stderr.contains("line 2, column 15"), "{}", run.stderr);
}

#[test]
fn help_lists_the_commands() {
    for flag in ["help", "--help", "-h"] {
        let run = semres(&[flag]);
        assert_eq!(run.code, 0);
        for command in ["resolve", "tree", "sort", "check", "explain", "outdated"] {
            assert!(run.stdout.contains(command), "{command} missing from help");
        }
    }
}

#[test]
fn library_entry_point_writes_to_the_given_streams() {
    let args = vec!["help".to_string()];
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(semver_resolver::cli::run(&args, &mut out, &mut err), 0);
    assert!(out.starts_with(b"semres:") && err.is_empty());

    let args = vec!["nonsense".to_string()];
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(semver_resolver::cli::run(&args, &mut out, &mut err), 2);
    assert!(out.is_empty() && err.starts_with(b"error: unknown command"));
}
