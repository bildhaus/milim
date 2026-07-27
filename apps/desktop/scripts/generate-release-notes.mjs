import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const appRoot = fileURLToPath(new URL("..", import.meta.url));
const repoRoot = resolve(appRoot, "..", "..");
const packageVersion = JSON.parse(readFileSync(resolve(appRoot, "package.json"), "utf8")).version;
const version = readFileSync(resolve(repoRoot, "VERSION"), "utf8").trim();
const releases = JSON.parse(readFileSync(resolve(appRoot, "src", "update", "releases.json"), "utf8"));
const release = releases[version];
const iconNames = new Set(["file-text", "github", "git-pull-request", "google", "plug"]);
const maxUpdateCards = 3;
const oversizedLegacyReleases = new Set(["0.2.2", "0.2.3"]);

for (const [releaseVersion, releaseEntry] of Object.entries(releases)) {
  if (!oversizedLegacyReleases.has(releaseVersion) && Array.isArray(releaseEntry?.items) && releaseEntry.items.length > maxUpdateCards) {
    fail(`Release ${releaseVersion} has ${releaseEntry.items.length} update cards; maximum is ${maxUpdateCards}.`);
  }
}

if (packageVersion !== version) fail(`VERSION ${version} does not match desktop package version ${packageVersion}.`);
if (!release || typeof release !== "object") fail(`Missing update-card release entry for ${version}.`);
if (release.version !== version) fail(`Release key ${version} must match its version field.`);
if (typeof release.summary !== "string" || !release.summary.trim()) fail(`Release ${version} needs a summary.`);
if (!Array.isArray(release.items) || release.items.length === 0) fail(`Release ${version} needs at least one item.`);

const ids = new Set();
for (const [index, item] of release.items.entries()) {
  for (const field of ["id", "eyebrow", "title", "description", "accent", "icon"]) {
    if (typeof item?.[field] !== "string" || !item[field].trim()) {
      fail(`Release ${version} item ${index + 1} needs ${field}.`);
    }
  }
  if (ids.has(item.id)) fail(`Release ${version} has duplicate item id ${item.id}.`);
  if (!iconNames.has(item.icon)) fail(`Release ${version} item ${item.id} has unknown icon ${item.icon}.`);
  if (!Array.isArray(item.details) || item.details.length === 0 || item.details.some((detail) => typeof detail !== "string" || !detail.trim())) {
    fail(`Release ${version} item ${item.id} needs details.`);
  }
  ids.add(item.id);
}

const markdown = [
  release.summary,
  "",
  ...release.items.flatMap((item) => [
    `## ${item.title}`,
    "",
    `_${item.eyebrow}_`,
    "",
    item.description,
    "",
    ...item.details.map((detail) => `- ${detail}`),
    "",
  ]),
].join("\n").trimEnd() + "\n";

const outputIndex = process.argv.indexOf("--output");
if (outputIndex >= 0) {
  const output = process.argv[outputIndex + 1];
  if (!output) fail("--output requires a file path.");
  writeFileSync(resolve(process.cwd(), output), markdown);
  console.log(`Release notes written for ${version}: ${output}`);
} else if (process.argv.includes("--check")) {
  console.log(`Release notes verified for ${version}.`);
} else {
  process.stdout.write(markdown);
}

function fail(message) {
  console.error(message);
  process.exit(1);
}
