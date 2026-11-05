# semver-resolver

A SemVer 2.0 library and a small command line tool, `semres`, that picks a consistent set of package versions from a JSON registry. It backtracks when a choice leads to a dead end, and when nothing fits it tells you which requirements clashed, who imposed them, and what it tried.

It has no external dependencies. That includes JSON: there is a small strict parser in `src/json.rs`.

## Install

You need a stable Rust toolchain.

```
cargo install --path .
```

Or build it in place with `cargo build --release` and use `target/release/semres`.

## Usage

A registry lists each package's releases and what they depend on:

```json
{
  "packages": {
    "wick": { "0.3.0": {}, "0.3.5": {} },
    "lantern": { "1.3.0": { "dependencies": { "wick": "~0.3" } } }
  }
}
```

A manifest names the project and what it needs. It can also hold `version` and `pins`:

```json
{ "name": "cabin", "version": "0.1.0", "dependencies": { "lantern": "^1", "marmot": ">=0.9" } }
```

The `examples/` folder has a slightly bigger pair. Resolve it and write a lockfile:

```
$ semres resolve -r examples/registry.json -m examples/manifest.json -l cabin.lock --write
resolved 5 packages in 9 steps, 4 backtracks
  bricks 1.4.2
  lantern 1.0.0
  marmot 1.0.0
  nest 1.1.0
  perch 1.0.0
```

The newest `lantern` needs `bricks ^2` and `marmot` needs `bricks ^1.4`, so the solver settled on the older `lantern`. Later runs with `-l cabin.lock` keep the locked versions as long as they still fit.

See the result as a tree, or in install order with cycles grouped:

```
$ semres tree -l cabin.lock
cabin 0.1.0
├── lantern 1.0.0 [^1]
│   └── bricks 1.4.2 [^1.0]
└── marmot 1.0.0 [>=0.9]
    ├── bricks 1.4.2 [^1.4]
    └── nest 1.1.0 [1.x]
        └── perch 1.0.0 [^1]
            └── nest 1.1.0 [^1] (cycle)

$ semres sort -l cabin.lock
bricks 1.4.2
lantern 1.0.0
cycle: nest 1.1.0, perch 1.0.0
marmot 1.0.0
```

When nothing fits, you get the reasons on stderr and exit code 1:

```
$ semres resolve -r examples/registry.json -m examples/conflict.json
cannot resolve: `bricks` 2.0.0 was selected, but a later requirement rejects it

requirements on bricks:
  ^2    from shed (root)
  ^1.4  from marmot 1.0.0
  combined: <none>

available versions:
  2.0.0  rejected by ^1.4 from marmot 1.0.0 (out of range)
  2.0.0-rc.1  rejected by ^2 from shed (root) (out of range)
  1.4.2  rejected by ^2 from shed (root) (out of range)
  1.0.0  rejected by ^2 from shed (root) (out of range)

selected when this failed: bricks 2.0.0, marmot 1.0.0

tried:
  bricks: 2.0.0
  marmot: 0.9.0, 1.0.0

search: 3 steps, 3 backtracks
```

The other commands:

- `semres check -r R -m M -l LOCK` verifies a lockfile against the manifest and registry.
- `semres explain <package> -r R -m M` says why a package is at its version and how each other release fared.
- `semres outdated -r R -l LOCK` lists locked packages with newer releases.

`semres help` lists every option. The solver takes `--prefer-lowest`, `--pin name=version`, `--pre`, `--pre-for name` and `--max-steps n`. Exit codes are 0 for success, 1 when the operation found a problem, and 2 for bad usage or unreadable input.

## Constraints

`^ ~ ~> >= > <= < =`, bare and partial versions (`1`, `1.2`), wildcards (`*`, `1.x`, `1.2.*`), hyphen ranges (`1.2 - 2.0`), spaces or commas for AND, and `||` for OR.

Prereleases follow the npm rule: `1.5.0-rc.1` only matches if a comparator in the same AND group names a prerelease of `1.5.0`. `--pre` lifts that rule, but derived ceilings still hold, so `^1` never admits `2.0.0-alpha`.

## Library

```rust
use semver_resolver::{resolve, Manifest, Options, Registry};

let registry = Registry::from_json(r#"{"packages": {"wick": {"0.3.5": {}}}}"#).unwrap();
let manifest = Manifest::from_json(r#"{"name": "cabin", "dependencies": {"wick": "~0.3"}}"#).unwrap();
let solved = resolve(&registry, &manifest, &Options::default()).unwrap();
println!("{}", solved.resolution.packages["wick"].version);
```

## Tests

```
cargo test
```

## Known limitations

- The search is plain backtracking with no backjumping or caching, so the worst case is exponential. The step budget (`--max-steps`, default 100000) stops it, and a budget stop is reported separately from a real conflict.
- The search recurses once per decision, so a very large graph uses a lot of stack.
- A conflict report describes the deepest failure the search reached. It is not a minimal proof that no solution exists, and it may name more requirements than are needed to explain the clash.
- `outdated` works out `wanted` from the constraints of the locked dependents only, and ignores the new version's own dependencies. In the example above it offers `lantern` 1.3.0, which cannot actually be resolved.
- Numeric version components and prerelease identifiers must fit in a `u64`. SemVer sets no such limit, so larger values are rejected.
- Two releases that differ only in build metadata count as duplicates, so a registry cannot hold both.
- Pins only apply to packages already in the graph. A pin on anything else is silently ignored.
- The root project is its own namespace. A registry package with the same name as the root is treated as a different package.
- `tree`, `sort` and `explain` trust the lockfile they are given. Run `check` if it may be stale.
- `check` and `outdated` accept `--pre` but not `--pre-for`.
- A hyphen range must stand alone in its AND group, so `1 - 2 <1.5` is an error.
- A dangling `||` is an error, where npm treats an empty alternative as `*`. Only a wholly empty string means `*`.
- With `--pre`, partial floors stay on the release: `1.x` does not admit `1.0.0-alpha`, while `>=1.0.0-0` does.
- There are no yanked releases, optional dependencies or feature flags.

## License

MIT
