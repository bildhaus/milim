import { mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const siteRoot = fileURLToPath(new URL("..", import.meta.url));
const repoRoot = join(siteRoot, "..", "..");
const docsRoot = join(repoRoot, "docs", "wiki");
const distRoot = join(siteRoot, "dist");
const ssrRoot = join(siteRoot, ".ssr-dist");
const internalPrefix = "__docs";
const templateHtml = readFileSync(join(distRoot, "index.html"), "utf8");
const { renderDocsPage, renderLandingPage } = await import(pathToFileURL(join(ssrRoot, "entry-server.js")).href);

const pages = readdirSync(docsRoot)
  .filter((name) => name.endsWith(".md"))
  .map((name) => parsePage(name, readFileSync(join(docsRoot, name), "utf8")))
  .sort((a, b) => a.path.localeCompare(b.path));

const paths = new Set();
for (const page of pages) {
  if (paths.has(page.path)) throw new Error(`Duplicate docs path: ${page.path || "/"}`);
  paths.add(page.path);

  const pathname = page.path ? `/${page.path}` : "/";
  const html = withMetadata(withBody(templateHtml, await renderDocsPage(pathname)), page);
  const outputDir = join(distRoot, internalPrefix, page.path);
  mkdirSync(outputDir, { recursive: true });
  writeFileSync(join(outputDir, "index.html"), html);
}

if (!paths.has("")) throw new Error("Docs frontmatter must define one root page with an empty path");

writeFileSync(join(distRoot, "index.html"), withBody(templateHtml, renderLandingPage()));
writeFileSync(join(distRoot, "sitemap.xml"), sitemap(pages));
writeFileSync(join(distRoot, "_worker.js"), workerSource(pages));
rmSync(ssrRoot, { recursive: true, force: true });
console.log(`Prerendered the landing page and ${pages.length} docs pages with static bodies.`);

function parsePage(name, markdown) {
  const match = markdown.match(/^---\r?\n([\s\S]*?)\r?\n---(?:\r?\n|$)/);
  if (!match) throw new Error(`${name} is missing frontmatter`);

  const meta = {};
  for (const line of match[1].split(/\r?\n/)) {
    const separator = line.indexOf(":");
    if (separator === -1) continue;
    meta[line.slice(0, separator).trim()] = line.slice(separator + 1).trim();
  }

  for (const key of ["id", "path", "title", "summary", "updated"]) {
    if (!(key in meta)) throw new Error(`${name} is missing frontmatter key ${key}`);
  }
  for (const key of ["id", "title", "summary", "updated"]) {
    if (!meta[key]) throw new Error(`${name} has an empty frontmatter key ${key}`);
  }
  if (meta.path && !/^[a-z0-9-]+(?:\/[a-z0-9-]+)*$/.test(meta.path)) {
    throw new Error(`${name} has unsafe docs path ${meta.path}`);
  }

  if (!/^\d{4}-\d{2}-\d{2}$/.test(meta.updated)) throw new Error(`${name} has an invalid updated date`);

  return { id: meta.id, path: meta.path, title: meta.title, summary: meta.summary, updated: meta.updated };
}

function withBody(html, body) {
  return replaceOne(html, /<div id="root"><\/div>/i, `<div id="root">${body}</div>`, "root");
}

function withMetadata(html, page) {
  const canonicalPath = page.path ? `/${page.path}` : "/";
  const canonicalUrl = `https://docs.milim.ai${canonicalPath}`;
  const title = page.id === "overview" ? "milim docs - Wiki" : `${page.title} - milim docs`;
  const escapedTitle = escapeAttribute(title);
  const escapedSummary = escapeAttribute(page.summary);
  const escapedUrl = escapeAttribute(canonicalUrl);

  return [
    [/<title>[\s\S]*?<\/title>/i, `<title>${escapeText(title)}</title>`, "title"],
    [/<meta\b(?=[^>]*\bname="description")[^>]*>/i, `<meta name="description" content="${escapedSummary}" />`, "description"],
    [/<link\b(?=[^>]*\brel="canonical")[^>]*>/i, `<link rel="canonical" href="${escapedUrl}" />`, "canonical"],
    [/<meta\b(?=[^>]*\bproperty="og:url")[^>]*>/i, `<meta property="og:url" content="${escapedUrl}" />`, "og:url"],
    [/<meta\b(?=[^>]*\bproperty="og:title")[^>]*>/i, `<meta property="og:title" content="${escapedTitle}" />`, "og:title"],
    [/<meta\b(?=[^>]*\bproperty="og:description")[^>]*>/i, `<meta property="og:description" content="${escapedSummary}" />`, "og:description"],
    [/<meta\b(?=[^>]*\bname="twitter:title")[^>]*>/i, `<meta name="twitter:title" content="${escapedTitle}" />`, "twitter:title"],
    [/<meta\b(?=[^>]*\bname="twitter:description")[^>]*>/i, `<meta name="twitter:description" content="${escapedSummary}" />`, "twitter:description"],
  ].reduce((output, [pattern, replacement, label]) => replaceOne(output, pattern, replacement, label), html);
}

function replaceOne(html, pattern, replacement, label) {
  const matches = html.match(new RegExp(pattern.source, `${pattern.flags}g`));
  if (matches?.length !== 1) throw new Error(`Expected one ${label} tag in built index.html, found ${matches?.length ?? 0}`);
  return html.replace(pattern, replacement);
}

function escapeText(value) {
  return value.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");
}

function escapeAttribute(value) {
  return escapeText(value).replaceAll('"', "&quot;").replaceAll("'", "&#39;");
}

function sitemap(docs) {
  const today = new Date().toISOString().slice(0, 10);
  const urls = [
    { loc: "https://milim.ai/", updated: today },
    ...docs.map((page) => ({
      loc: `https://docs.milim.ai${page.path ? `/${page.path}` : "/"}`,
      updated: page.updated,
    })),
  ];

  return `<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
${urls.map(({ loc, updated }) => `  <url>
    <loc>${loc}</loc>
    <lastmod>${updated}</lastmod>
  </url>`).join("\n")}
</urlset>
`;
}

function workerSource(docs) {
  const routes = docs.map((page) => page.path ? `/${page.path}` : "/");

  return `const DOCS_HOST = "docs.milim.ai";
const MAIN_HOST = "milim.ai";
const INTERNAL_PREFIX = "/${internalPrefix}";
const DOC_PATHS = new Set(${JSON.stringify(routes)});

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const hostname = url.hostname.toLowerCase();
    const pathname = normalizePath(url.pathname);

    if (pathname === INTERNAL_PREFIX || pathname.startsWith(\`\${INTERNAL_PREFIX}/\`)) {
      return new Response("Not found", { status: 404 });
    }

    if (hostname === \`www.\${MAIN_HOST}\`) return redirect(url, MAIN_HOST, url.pathname);

    const legacyDocsPath = stripDocsPrefix(pathname);
    if (hostname === MAIN_HOST && legacyDocsPath !== null) {
      return redirect(url, DOCS_HOST, legacyDocsPath);
    }
    if (hostname === DOCS_HOST) {
      if (legacyDocsPath !== null) return redirect(url, DOCS_HOST, legacyDocsPath);
      return serveDocs(request, env, url, pathname);
    }
    if (legacyDocsPath !== null) return serveDocs(request, env, url, legacyDocsPath);

    return env.ASSETS.fetch(request);
  },
};

function normalizePath(pathname) {
  return pathname.replace(/\\/+$/, "") || "/";
}

function stripDocsPrefix(pathname) {
  const match = pathname.match(/^\\/(?:docs|wiki)(?:\\/(.*))?$/);
  return match ? (match[1] ? \`/\${match[1]}\` : "/") : null;
}

function redirect(url, hostname, pathname) {
  const target = new URL(url);
  target.protocol = "https:";
  target.hostname = hostname;
  target.port = "";
  target.pathname = pathname;
  return Response.redirect(target, 301);
}

function serveDocs(request, env, url, pathname) {
  if (!DOC_PATHS.has(pathname)) return env.ASSETS.fetch(request);

  const assetUrl = new URL(url);
  assetUrl.hostname = "assets.local";
  assetUrl.pathname = pathname === "/" ? \`\${INTERNAL_PREFIX}/\` : \`\${INTERNAL_PREFIX}\${pathname}/\`;
  return env.ASSETS.fetch(new Request(assetUrl, request));
}
`;
}
