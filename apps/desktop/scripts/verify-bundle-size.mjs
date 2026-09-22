// Fails when the JavaScript or CSS that dist/index.html loads at startup grows
// past its budget. Run after `vite build`. Budgets sit roughly 10% above the
// measured sizes; raise them deliberately when the growth is intended.
import { existsSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const appRoot = fileURLToPath(new URL("..", import.meta.url));
const distDir = join(appRoot, "dist");
const indexPath = join(distDir, "index.html");

// Uncompressed bytes. Measured at v0.2.68: initial JS 552,533 B, initial CSS 237,874 B.
const budgets = {
  js: 608_000,
  css: 262_000,
};

if (!existsSync(indexPath)) {
  fail(`${indexPath} not found; run \`vite build\` first.`);
}

const html = readFileSync(indexPath, "utf8");
const tags = [...html.matchAll(/<(script|link)\b[^>]*>/gi)].map((match) => match[0]);
const initial = { js: [], css: [] };
for (const tag of tags) {
  const src = attribute(tag, "src");
  const href = attribute(tag, "href");
  const rel = attribute(tag, "rel")?.toLowerCase();
  if (tag.startsWith("<script") && src && attribute(tag, "type") === "module") initial.js.push(src);
  else if (rel === "modulepreload" && href) initial.js.push(href);
  else if (rel === "stylesheet" && href) initial.css.push(href);
}

if (!initial.js.length) fail("dist/index.html does not reference an entry module script.");
if (!initial.css.length) fail("dist/index.html does not reference an initial stylesheet.");

let failed = false;
for (const [kind, label] of [
  ["js", "Initial JS (entry + modulepreload)"],
  ["css", "Initial CSS"],
]) {
  const files = initial[kind].map((url) => ({ url, bytes: statSync(localPath(url)).size }));
  const total = files.reduce((sum, file) => sum + file.bytes, 0);
  const budget = budgets[kind];
  const status = total <= budget ? "ok" : "OVER BUDGET";
  console.log(`${label}: ${formatBytes(total)} / ${formatBytes(budget)} ${status}`);
  for (const file of files) console.log(`  ${file.url} ${formatBytes(file.bytes)}`);
  if (total > budget) failed = true;
}

if (failed) {
  fail("Bundle size budget exceeded. Lazy-load the new code or raise the budget in scripts/verify-bundle-size.mjs deliberately.");
}

function attribute(tag, name) {
  const match = new RegExp(`\\s${name}\\s*=\\s*(?:"([^"]*)"|'([^']*)'|([^\\s>]+))`, "i").exec(tag);
  return match ? (match[1] ?? match[2] ?? match[3]) : undefined;
}

function localPath(url) {
  const path = join(distDir, url.replace(/^\.?\//, "").split(/[?#]/)[0]);
  if (!existsSync(path)) fail(`dist/index.html references missing file ${url}`);
  return path;
}

function formatBytes(bytes) {
  return `${(bytes / 1024).toFixed(1)} KiB`;
}

function fail(message) {
  console.error(message);
  process.exit(1);
}
