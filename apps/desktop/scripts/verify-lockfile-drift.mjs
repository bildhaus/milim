// The Tauri app (apps/desktop/src-tauri) is its own Cargo workspace with its
// own Cargo.lock, but it links the shared milim-* crates from the root
// workspace. `cargo test` at the root therefore exercises those crates against
// the root lockfile while the shipped app resolves them through the Tauri
// lockfile. Fail when a crate in the dependency closure of the shared milim-*
// crates resolves to different versions in the two lockfiles.
//
// Fix drift with `cargo update -p <crate> --precise <version>` in the lockfile
// that lags, or by running the same `cargo update -p <crate>` in both.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const appRoot = fileURLToPath(new URL("..", import.meta.url));
const repoRoot = join(appRoot, "..", "..");
const rootLock = parseLock(join(repoRoot, "Cargo.lock"));
const tauriLock = parseLock(join(appRoot, "src-tauri", "Cargo.lock"));

const isShared = (pkg) => !pkg.source && pkg.name.startsWith("milim-") && pkg.name !== "milim-desktop";
const rootClosure = closure(rootLock, rootLock.packages.filter(isShared));
const tauriClosure = closure(tauriLock, tauriLock.packages.filter(isShared));

const drift = [];
for (const [name, rootVersions] of rootClosure) {
  const tauriVersions = tauriClosure.get(name);
  if (!tauriVersions) continue;
  const left = [...rootVersions].sort().join(", ");
  const right = [...tauriVersions].sort().join(", ");
  if (left !== right) drift.push(`  ${name}: Cargo.lock ${left}; apps/desktop/src-tauri/Cargo.lock ${right}`);
}

if (drift.length) {
  console.error(
    `Shared milim-* dependencies resolve differently in the root and Tauri lockfiles:\n${drift.sort().join("\n")}`,
  );
  process.exit(1);
}
console.log(`Lockfiles agree on ${rootClosure.size} crates used by the shared milim-* crates.`);

function parseLock(path) {
  const packages = [];
  let current = null;
  let inDependencies = false;
  for (const raw of readFileSync(path, "utf8").split(/\r?\n/)) {
    const line = raw.trim();
    if (line === "[[package]]") {
      current = { name: "", version: "", source: "", dependencies: [] };
      packages.push(current);
      inDependencies = false;
    } else if (!current) {
      continue;
    } else if (inDependencies) {
      if (line === "]") inDependencies = false;
      else if (line.startsWith('"')) current.dependencies.push(line.replace(/^"|",?$/g, ""));
    } else if (line.startsWith("dependencies = [")) {
      inDependencies = !line.endsWith("]");
    } else {
      const match = /^(name|version|source) = "(.*)"$/.exec(line);
      if (match) current[match[1]] = match[2];
    }
  }
  const byName = new Map();
  for (const pkg of packages) {
    if (!byName.has(pkg.name)) byName.set(pkg.name, []);
    byName.get(pkg.name).push(pkg);
  }
  return { packages, byName };
}

// Lockfile dependency entries are "name" when only one version is locked, or
// "name version" / "name version (source)" when several are.
function resolveDependency(lock, entry) {
  const [name, version] = entry.split(" ");
  const candidates = lock.byName.get(name) ?? [];
  const pkg = version ? candidates.find((candidate) => candidate.version === version) : candidates[0];
  if (!pkg) throw new Error(`Cannot resolve lockfile dependency ${entry}`);
  return pkg;
}

function closure(lock, roots) {
  const versions = new Map();
  const seen = new Set();
  const queue = [...roots];
  while (queue.length) {
    const pkg = queue.pop();
    const key = `${pkg.name} ${pkg.version} ${pkg.source}`;
    if (seen.has(key)) continue;
    seen.add(key);
    if (pkg.source) {
      if (!versions.has(pkg.name)) versions.set(pkg.name, new Set());
      versions.get(pkg.name).add(pkg.version);
    }
    for (const entry of pkg.dependencies) queue.push(resolveDependency(lock, entry));
  }
  return versions;
}
