import { spawnSync } from "node:child_process";
import { rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { expect, expectOnlyChanged, finish, hasCargo, headFile, read, repo, skip, stripComments } from "../../lib/check.mjs";

if (!hasCargo()) skip("cargo is not installed");

// Two bugs: a borrow error (yank_below calls a `&mut self` method while
// iterating `self.releases` mutably) and a derived `Ord` that compares the
// fields in declaration order, patch first. The visible test only sorts;
// the hidden tests also compare versions directly and pick `latest`.
for (const path of ["tests/registry.rs", "Cargo.toml"]) {
  expect(read(path) === headFile(path), `${path} was modified`);
}
expectOnlyChanged(["src/lib.rs", "Cargo.lock"]);
expect(!/\bunsafe\b/.test(stripComments(read("src/lib.rs"))), "src/lib.rs uses unsafe");

const HIDDEN = String.raw`
use release_registry::{Registry, Version};

fn registry(versions: &[&str]) -> Registry {
    let mut registry = Registry::new();
    for version in versions {
        registry.publish(version, "notes").unwrap();
    }
    registry
}

#[test]
fn eval_versions_compare_major_minor_patch() {
    assert!(Version::new(1, 2, 3) < Version::new(2, 0, 0));
    assert!(Version::new(1, 9, 9) < Version::new(1, 10, 0));
    assert!(Version::new(1, 10, 0) < Version::new(1, 10, 1));
    assert_eq!(Version::new(3, 1, 4).max(Version::new(3, 0, 9)), Version::new(3, 1, 4));
}

#[test]
fn eval_latest_is_the_highest_unyanked_version() {
    let mut registry = registry(&["2.0.0", "1.4.7", "2.1.0", "1.10.2"]);
    assert_eq!(registry.latest().unwrap().version, Version::new(2, 1, 0));
    registry.yank("2.1.0").unwrap();
    assert_eq!(registry.latest().unwrap().version, Version::new(2, 0, 0));
}

#[test]
fn eval_yank_below_logs_each_yank() {
    let mut registry = registry(&["1.0.9", "0.3.0", "1.1.0", "1.0.10"]);
    registry.yank("0.3.0").unwrap();
    assert_eq!(registry.yank_below("1.1.0"), Ok(2));
    let log = registry.log();
    assert_eq!(log.len(), 7, "{log:?}");
    assert!(log[5..].contains(&"yanked 1.0.9".to_string()), "{log:?}");
    assert!(log[5..].contains(&"yanked 1.0.10".to_string()), "{log:?}");
    assert_eq!(registry.latest().unwrap().version, Version::new(1, 1, 0));
}
`;

const hidden = join(repo, "tests", "eval_hidden.rs");
writeFileSync(hidden, HIDDEN);
try {
  const result = spawnSync("cargo", ["test", "--offline", "--quiet"], {
    cwd: repo,
    encoding: "utf8",
    timeout: 110_000,
    env: { ...process.env, CARGO_TERM_COLOR: "never" },
  });
  if (result.status !== 0) {
    const output = `${result.stdout}\n${result.stderr}`;
    const reasons = output
      .split("\n")
      .filter((line) => /^error(\[E\d+\])?:|panicked at|^test .* FAILED|assertion/.test(line.trim()))
      .slice(0, 6)
      .join(" | ");
    expect(false, `cargo test (with hidden tests) failed: ${reasons || `exit ${result.status}`}`);
  }
} finally {
  rmSync(hidden, { force: true });
}

finish();
