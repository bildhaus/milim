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

const expectedArtifacts = [
  { artifact: "macos-universal", os: "macos-latest", args: "--target universal-apple-darwin --bundles app,dmg" },
  { artifact: "windows-x64", os: "windows-latest", args: "" },
];

const checkoutReleaseTagRef = "ref: ${{ env.MILIM_RELEASE_TAG }}";
const verifyCommand = "pnpm -C apps/desktop verify";

for (const artifact of expectedArtifacts) {
  assertIncludes(releaseWorkflow, matrixRow(artifact), "release workflow matrix");
}

for (const needle of [
  "create-draft-release:",
  "Create or update draft release",
  'gh release create "${MILIM_RELEASE_TAG}"',
  'gh release edit "${MILIM_RELEASE_TAG}"',
  "node scripts/generate-release-notes.mjs --output release-notes.md",
  "--notes-file apps/desktop/release-notes.md",
  "--verify-tag",
  "needs: create-draft-release",
  "Checkout release tag",
  checkoutReleaseTagRef,
  "GITHUB_REF_TYPE: tag",
  "GITHUB_REF_NAME: ${{ env.MILIM_RELEASE_TAG }}",
  verifyCommand,
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
  'gh release upload "${MILIM_RELEASE_TAG}" release-published/manifest.json release-published/SHA256SUMS.txt --repo "${GITHUB_REPOSITORY}" --clobber',
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
  "tester-artifacts",
  "verify-release-download-set",
  "verify-downloaded-release-artifact",
  "QA_EVIDENCE",
  "HANDOFF.md",
  "tauri-apps/tauri-action@v0",
  'gh release download "${MILIM_RELEASE_TAG}" --repo "${GITHUB_REPOSITORY}" --pattern "*.sha256" --dir release-checksums',
  "cat release-checksums/*.sha256 | sort -k2 > SHA256SUMS.txt",
  'gh release upload "${{ env.MILIM_RELEASE_TAG }}" src-tauri/target/release/bundle/portable/*.exe --clobber',
  'gh release upload "${{ env.MILIM_RELEASE_TAG }}" src-tauri/target/release/bundle/portable/*.exe.sha256 --clobber',
]) {
  assertNotIncludes(releaseWorkflow, needle, "release workflow");
}

assertBefore(releaseWorkflow, "Validate release tag", checkoutReleaseTagRef, "release workflow checkout");
assertBefore(releaseWorkflow, "Generate release notes", "Create or update draft release", "release workflow notes");
assertBefore(releaseWorkflow, checkoutReleaseTagRef, "Require macOS signing secrets", "release workflow signing preflight");
assertBefore(releaseWorkflow, "Require macOS signing secrets", verifyCommand, "release workflow signing preflight");
assertBefore(releaseWorkflow, "Require macOS signing secrets", "Build macOS app and DMG", "release workflow signing preflight");
assertBefore(releaseWorkflow, checkoutReleaseTagRef, verifyCommand, "release workflow verify");
assertBefore(releaseWorkflow, verifyCommand, "Build macOS app and DMG", "release workflow verify");
assertBefore(releaseWorkflow, verifyCommand, "pnpm -C apps/desktop tauri build --no-bundle", "release workflow verify");
assertBefore(releaseWorkflow, "pnpm -C apps/desktop tauri build --no-bundle", "node scripts/smoke-release-binary.mjs", "release workflow launch smoke");
assertBefore(releaseWorkflow, "node scripts/stage-portable-release-artifact.mjs", "node scripts/generate-release-manifest.mjs", "release workflow manifest");
assertBefore(releaseWorkflow, "node scripts/generate-release-manifest.mjs", "node scripts/verify-release-manifest.mjs", "release workflow manifest");
assertBefore(releaseWorkflow, "node scripts/verify-release-manifest.mjs", "Upload release manifest artifact", "release workflow manifest");
assertBefore(releaseWorkflow, "desktop:", "publish-release-checksums:", "release workflow checksums");

for (const needle of [
  'tags: ["v*"]',
  "cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings",
  "cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml",
]) {
  assertIncludes(ciWorkflow, needle, "CI workflow");
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

function assertBefore(text, first, second, label) {
  const firstIndex = text.indexOf(first);
  const secondIndex = text.indexOf(second);
  if (firstIndex < 0) throw new Error(`${label} must include ${first}`);
  if (secondIndex < 0) throw new Error(`${label} must include ${second}`);
  if (firstIndex >= secondIndex) throw new Error(`${label} must run ${first} before ${second}`);
}
