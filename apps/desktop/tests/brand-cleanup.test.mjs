import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";

const repoRoot = new URL("../../../", import.meta.url);
const compactOldBrand = ["wor", "de"].join("");
const splitOldBrand = ["word", "e"].join("_");
const oldBrandPattern = new RegExp(`${compactOldBrand}|${splitOldBrand}`, "i");

const files = execFileSync("git", ["ls-files"], {
  cwd: repoRoot,
  encoding: "utf8",
}).trim().split(/\r?\n/).filter(Boolean);

const matches = [];
for (const file of files) {
  const path = new URL(file.replaceAll("\\", "/"), repoRoot);
  if (!existsSync(path)) continue;
  const text = readFileSync(path, "utf8");
  const lines = text.split(/\r?\n/);
  lines.forEach((line, index) => {
    if (oldBrandPattern.test(line)) {
      matches.push(`${file}:${index + 1}: ${line.trim()}`);
    }
  });
}

if (matches.length > 0) {
  throw new Error(`Old brand references remain:\n${matches.join("\n")}`);
}

// The product name is lowercase "milim" in user-facing copy. Scan desktop
// source lines outside comments and imports for a standalone capitalized
// "Milim"; identifiers such as MilimIcon are not standalone words.
const capitalizedBrandPattern = /(^|[^A-Za-z0-9_$])Milim(?![A-Za-z0-9_$])/;
const desktopSourcePattern = /^apps\/desktop\/src\/.+\.tsx?$/;
const skippedSourcePattern = /(\.test\.tsx?$|\.d\.ts$|\/generated\/|\/__tests__\/)/;
const skippedLinePattern = /^\s*(\/\/|\/\*|\*|import\b|export\s.*\bfrom\s)/;

const capitalizedMatches = [];
for (const file of files) {
  if (!desktopSourcePattern.test(file) || skippedSourcePattern.test(file)) continue;
  const path = new URL(file, repoRoot);
  if (!existsSync(path)) continue;
  readFileSync(path, "utf8").split(/\r?\n/).forEach((line, index) => {
    if (!skippedLinePattern.test(line) && capitalizedBrandPattern.test(line)) {
      capitalizedMatches.push(`${file}:${index + 1}: ${line.trim()}`);
    }
  });
}

if (capitalizedMatches.length > 0) {
  throw new Error(`User-facing copy must write the product name as lowercase "milim":\n${capitalizedMatches.join("\n")}`);
}
