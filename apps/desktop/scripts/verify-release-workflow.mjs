import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const appRoot = fileURLToPath(new URL("..", import.meta.url));
const repoRoot = join(appRoot, "..", "..");
const releaseWorkflow = readFileSync(
  process.env.MILIM_RELEASE_WORKFLOW_PATH || join(repoRoot, ".github", "workflows", "release.yml"),
  "utf8",
);
const ciWorkflow = readFileSync(process.env.MILIM_CI_WORKFLOW_PATH || join(repoRoot, ".github", "workflows", "ci.yml"), "utf8");
const siteWorkflow = readFileSync(
  process.env.MILIM_SITE_WORKFLOW_PATH || join(repoRoot, ".github", "workflows", "site.yml"),
  "utf8",
);
const desktopPackage = JSON.parse(readFileSync(join(appRoot, "package.json"), "utf8"));
const runtimeEvidenceJobStart = releaseWorkflow.indexOf("\n  runtime-evidence:");
const runtimeEvidenceJobEnd = releaseWorkflow.indexOf("\n  desktop:", runtimeEvidenceJobStart);
if (runtimeEvidenceJobStart < 0 || runtimeEvidenceJobEnd < 0) throw new Error("Release workflow must include runtime-evidence before desktop packaging");
const runtimeEvidenceJob = releaseWorkflow.slice(runtimeEvidenceJobStart, runtimeEvidenceJobEnd);

const expectedArtifacts = [
  { artifact: "macos-universal", os: "macos-latest", args: "--target universal-apple-darwin --bundles app,dmg" },
  { artifact: "windows-x64", os: "windows-latest", args: "" },
];

const checkoutReleaseTagRef = "ref: ${{ env.MILIM_RELEASE_TAG }}";

for (const artifact of expectedArtifacts) {
  assertIncludes(releaseWorkflow, matrixRow(artifact), "release workflow matrix");
}

for (const needle of [
  "create-draft-release:",
  "Create or update draft release",
  'gh release create "${MILIM_RELEASE_TAG}"',
  'gh release edit "${MILIM_RELEASE_TAG}"',
  "--json isDraft --jq",
  "Refusing to edit published release",
  "Refusing to upload assets to published release",
  "node scripts/generate-release-notes.mjs --output release-notes.md",
  "--notes-file apps/desktop/release-notes.md",
  "--verify-tag",
  "needs: create-draft-release",
  "Checkout release tag",
  checkoutReleaseTagRef,
  "save-if: false",
  "Require macOS signing secrets",
  "::error::Missing required macOS signing secret",
  "Build macOS app and DMG",
  "pnpm -C apps/desktop tauri build ${{ matrix.args }}",
  "pnpm -C apps/desktop tauri build --no-bundle",
  "node scripts/smoke-release-binary.mjs",
  "node scripts/stage-portable-release-artifact.mjs",
  "node scripts/generate-release-manifest.mjs",
  "node scripts/verify-release-manifest.mjs",
  "actions/upload-artifact@v4",
  "release-manifest-${{ matrix.artifact }}",
  "updater-checksums-${{ matrix.artifact }}",
  "actions/download-artifact@v4",
  "node apps/desktop/scripts/merge-release-manifests.mjs release-manifests release-published",
  "milim-windows-x64-portable.exe",
  "milim-macos-universal.dmg",
  "cat release-checksums/*.sha256 | sort -k2 > release-published/SHA256SUMS.txt",
  'gh release upload "${MILIM_RELEASE_TAG}" release-published/manifest.json release-published/SHA256SUMS.txt --repo "${GITHUB_REPOSITORY}"',
]) {
  assertIncludes(releaseWorkflow, needle, "release workflow");
}

for (const needle of [
  "ubuntu-latest, args:",
  "linux-x64",
  "--include-linux",
  "verify:native-prompt",
  "verify:native-vad",
  "verify:native-tts",
  "verify-release-download-set",
  "verify-downloaded-release-artifact",
  "QA_EVIDENCE",
  "HANDOFF.md",
  "tauri-apps/tauri-action@v0",
  'gh release download "${MILIM_RELEASE_TAG}" --repo "${GITHUB_REPOSITORY}" --pattern "*.sha256" --dir release-checksums',
  "cat release-checksums/*.sha256 | sort -k2 > SHA256SUMS.txt",
  'gh release upload "${{ env.MILIM_RELEASE_TAG }}" src-tauri/target/release/bundle/portable/*.exe --clobber',
  'gh release upload "${{ env.MILIM_RELEASE_TAG }}" src-tauri/target/release/bundle/portable/*.exe.sha256 --clobber',
  "--clobber",
]) {
  assertNotIncludes(releaseWorkflow, needle, "release workflow");
}
assertLineOccurrences(releaseWorkflow, "      - run: pnpm -C apps/desktop verify", 0, "release workflow");

assertEveryMutationGuarded(releaseWorkflow, "gh release edit", "--json isDraft --jq", "release edits");
assertEveryMutationGuarded(releaseWorkflow, "gh release upload", "--json isDraft --jq", "release uploads");
assertOccurrences(siteWorkflow, '- "VERSION"', 2, "site workflow version trigger");

assertBefore(releaseWorkflow, "Validate release tag", checkoutReleaseTagRef, "release workflow checkout");
assertBefore(releaseWorkflow, "Generate release notes", "Create or update draft release", "release workflow notes");
assertBefore(releaseWorkflow, checkoutReleaseTagRef, "Require macOS signing secrets", "release workflow signing preflight");
assertBefore(releaseWorkflow, "Require macOS signing secrets", "Build macOS app and DMG", "release workflow signing preflight");
assertBefore(releaseWorkflow, "pnpm -C apps/desktop tauri build --no-bundle", "node scripts/smoke-release-binary.mjs", "release workflow launch smoke");
assertBefore(releaseWorkflow, "node scripts/stage-portable-release-artifact.mjs", "node scripts/generate-release-manifest.mjs", "release workflow manifest");
assertBefore(releaseWorkflow, "node scripts/generate-release-manifest.mjs", "node scripts/verify-release-manifest.mjs", "release workflow manifest");
assertBefore(releaseWorkflow, "node scripts/verify-release-manifest.mjs", "Upload release manifest artifact", "release workflow manifest");
assertBefore(releaseWorkflow, "desktop:", "publish-release-checksums:", "release workflow checksums");
assertIncludes(
  siteWorkflow,
  "if: github.event_name == 'pull_request' || github.ref != 'refs/heads/main'",
  "site build workflow",
);
assertIncludes(
  siteWorkflow,
  "if: github.event_name != 'pull_request' && github.ref == 'refs/heads/main'",
  "site deploy workflow",
);
assertNotIncludes(siteWorkflow, "needs: check", "site workflow");

for (const needle of [
  "pull_request:",
  "cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings",
  "cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml",
]) {
  assertIncludes(ciWorkflow, needle, "CI workflow");
}
for (const line of [
  "  pull_request:",
  "  workflow_dispatch:",
]) {
  assertLineOccurrences(ciWorkflow, line, 1, "CI workflow trigger");
}
assertNotIncludes(ciWorkflow, 'tags: ["v*"]', "CI workflow");
assertNotIncludes(ciWorkflow, "runtime-evidence:", "CI workflow");
assertLineOccurrences(releaseWorkflow, '    tags: ["v*"]', 1, "Release workflow trigger");
assertOccurrences(releaseWorkflow, "tester-artifacts", 3, "Release runtime evidence paths");

assertEqual(
  desktopPackage.scripts["verify:runtime-conformance"],
  "node tests/runtime-conformance.mjs",
  "runtime conformance script",
);
assertEqual(
  desktopPackage.scripts["perf:canonical"],
  "npm run verify:tauri && node tests/tauri-dev-perf.mjs --binary",
  "canonical benchmark script",
);
assertEqual(desktopPackage.scripts["perf:tauri-dev"], "node tests/tauri-dev-perf.mjs", "Tauri dev benchmark script");
assertNotIncludes(desktopPackage.scripts.verify, "verify:runtime-conformance", "default desktop verification");
assertNotIncludes(desktopPackage.scripts.verify, "perf:canonical", "default desktop verification");

for (const needle of [
  "name: Runtime evidence (Windows)",
  "runs-on: windows-2022",
  "permissions:\n      contents: read",
  checkoutReleaseTagRef,
  "pnpm -C apps/desktop install --frozen-lockfile",
  "uses: actions/upload-artifact@v4",
]) {
  assertIncludes(runtimeEvidenceJob, needle, "runtime evidence job");
}
for (const line of [
  "        run: pnpm -C apps/desktop verify:runtime-conformance",
  "          MILIM_CONFORMANCE_ARTIFACT_DIR: ${{ github.workspace }}/apps/desktop/tester-artifacts/runtime-evidence",
  "        run: pnpm -C apps/desktop perf:canonical",
  "          MILIM_PERF_ARTIFACT_DIR: ${{ github.workspace }}/apps/desktop/tester-artifacts/runtime-evidence",
  "        uses: actions/upload-artifact@v4",
  "          name: runtime-evidence-windows",
  "          path: apps/desktop/tester-artifacts/runtime-evidence",
  "          if-no-files-found: error",
]) {
  assertLineOccurrences(runtimeEvidenceJob, line, 1, "runtime evidence job");
}
assertLineOccurrences(runtimeEvidenceJob, "        if: always()", 2, "runtime evidence job");

assertBefore(
  runtimeEvidenceJob,
  "pnpm -C apps/desktop verify:runtime-conformance",
  "pnpm -C apps/desktop perf:canonical",
  "runtime evidence job",
);
assertBefore(
  runtimeEvidenceJob,
  "pnpm -C apps/desktop perf:canonical",
  "actions/upload-artifact@v4",
  "runtime evidence job",
);

for (const needle of ["branches: [main]", "Optional feature smoke"]) {
  assertNotIncludes(ciWorkflow, needle, "CI workflow");
}

console.log(`Release workflow smoke verified: ${expectedArtifacts.map((artifact) => `milim-${artifact.artifact}`).join(", ")}`);

function matrixRow({ os, args, artifact }) {
  return `- { os: ${os}, args: "${args}", artifact: "${artifact}" }`;
}

function assertIncludes(text, needle, label) {
  if (!text.includes(needle)) throw new Error(`${label} must include ${needle}`);
}

function assertNotIncludes(text, needle, label) {
  if (text.includes(needle)) throw new Error(`${label} must not include ${needle}`);
}

function assertEqual(actual, expected, label) {
  if (actual !== expected) throw new Error(`${label} must equal ${expected}`);
}

function assertLineOccurrences(text, line, count, label) {
  const actual = text.split(/\r?\n/).filter((candidate) => candidate === line).length;
  if (actual !== count) {
    throw new Error(`${label} must contain ${count} exact line(s) ${line}; found ${actual}`);
  }
}

function assertBefore(text, first, second, label) {
  const firstIndex = text.indexOf(first);
  const secondIndex = text.indexOf(second);
  if (firstIndex < 0) throw new Error(`${label} must include ${first}`);
  if (secondIndex < 0) throw new Error(`${label} must include ${second}`);
  if (firstIndex >= secondIndex) throw new Error(`${label} must run ${first} before ${second}`);
}

function assertEveryMutationGuarded(text, mutation, guard, label) {
  let count = 0;
  let index = text.indexOf(mutation);
  while (index >= 0) {
    const stepStart = text.lastIndexOf("\n      - name:", index);
    const guardIndex = text.lastIndexOf(guard, index);
    if (guardIndex < stepStart) throw new Error(`${label} must check draft status before ${mutation}`);
    count += 1;
    index = text.indexOf(mutation, index + mutation.length);
  }
  if (!count) throw new Error(`${label} must include ${mutation}`);
}

function assertOccurrences(text, needle, expected, label) {
  const count = text.split(needle).length - 1;
  if (count !== expected) throw new Error(`${label} must include ${needle} exactly ${expected} times (found ${count})`);
}
