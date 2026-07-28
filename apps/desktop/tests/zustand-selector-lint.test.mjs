import { readdirSync, readFileSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { extname, join, sep } from "node:path";

const root = fileURLToPath(new URL("..", import.meta.url));
const src = join(root, "src");
const findings = [];
const selectorCallsGetSettings =
  /useSessions\s*\(\s*\(?\s*([A-Za-z_$][\w$]*)\)?\s*=>[\s\S]{0,300}?\1\.getSettings\s*\(/g;
const metadataOnlyFullSessionSelector =
  /useSessions\s*\(\s*\(?\s*([A-Za-z_$][\w$]*)\)?\s*=>\s*\1\.sessions\s*\)/g;

function files(path) {
  const stat = statSync(path);
  if (stat.isFile()) return [".ts", ".tsx"].includes(extname(path)) ? [path] : [];
  return readdirSync(path).flatMap((name) => files(join(path, name)));
}

for (const file of files(src)) {
  const text = readFileSync(file, "utf8");
  for (const match of text.matchAll(selectorCallsGetSettings)) {
    const line = text.slice(0, match.index).split(/\r?\n/).length;
    findings.push(`${file}:${line}: do not call getSettings inside a useSessions selector`);
  }
  if (
    file.endsWith(`${join("components", "Sidebar.tsx")}`) ||
    file.endsWith(`${join("components", "ChatView.tsx")}`) ||
    file.includes(`${join("components", "chat")}${sep}`)
  ) {
    for (const match of text.matchAll(metadataOnlyFullSessionSelector)) {
      const line = text.slice(0, match.index).split(/\r?\n/).length;
      findings.push(`${file}:${line}: metadata views must subscribe to summaries, not full sessions`);
    }
  }
}

if (findings.length) {
  throw new Error(`Unstable Zustand selectors found:\n${findings.join("\n")}`);
}
