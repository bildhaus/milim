import { useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { useGSAP } from "@gsap/react";
import gsap from "gsap";
import { ScrollTrigger } from "gsap/ScrollTrigger";
import versionRaw from "../../../VERSION?raw";
import { Eyebrow } from "./Eyebrow";
import { HeroAsciiField } from "./HeroAsciiField";
import { ShaderField } from "./ShaderField";
import { SiteMobileNav } from "./SiteMobileNav";
import { ThemeControl } from "./ThemeControl";

gsap.registerPlugin(useGSAP, ScrollTrigger);

const GITHUB_URL = "https://github.com/oshtz/milim";
const RELEASES_URL = `${GITHUB_URL}/releases/latest`;
const WINDOWS_URL = `${RELEASES_URL}/download/milim-windows-x64-portable.exe`;
const MACOS_URL = `${RELEASES_URL}/download/milim-macos-universal.dmg`;
const DOCS_VERSION = versionRaw.trim();
const docsNavLinks = [
  { label: "Home", href: "https://milim.ai/" },
  { label: "Product", href: "https://milim.ai/#product" },
  { label: "Releases", href: "https://milim.ai/#releases" },
  { label: "GitHub", href: GITHUB_URL },
];

const docModules = import.meta.glob("../../../docs/wiki/*.md", {
  eager: true,
  import: "default",
  query: "?raw",
}) as Record<string, string>;

type DocPageId = string;
type TocGroup = "Start" | "Core" | "Local data" | "Reference";

type DocMeta = {
  id: DocPageId;
  path: string;
  label: string;
  title: string;
  summary: string;
  group: TocGroup;
  order: number;
  updated: string;
};

type MarkdownBlock =
  | { type: "heading"; level: number; id: string; text: string }
  | { type: "paragraph"; text: string }
  | { type: "list"; items: string[] }
  | { type: "table"; headers: string[]; rows: string[][] }
  | { type: "code"; language: string; title: string; code: string }
  | { type: "quote"; title: string; body: string };

type DocPage = DocMeta & {
  blocks: MarkdownBlock[];
  searchSections: SearchDoc[];
};

type SearchDoc = {
  id: string;
  pageId: DocPageId;
  anchor: string;
  title: string;
  summary: string;
  text: string;
};

type SearchIndex = {
  docs: Array<SearchDoc & { tokens: string[]; titleTokens: string[] }>;
  avgLength: number;
  docFreq: Map<string, number>;
};

const tocOrder: TocGroup[] = ["Start", "Core", "Local data", "Reference"];
const docPages = Object.entries(docModules)
  .map(([path, raw]) => parseDocPage(path, raw))
  .sort((a, b) => a.order - b.order);
const docsPageById = new Map(docPages.map((page) => [page.id, page]));
const tocGroups: Array<[TocGroup, DocPageId[]]> = tocOrder
  .map((group) => [group, docPages.filter((page) => page.group === group).map((page) => page.id)] as [TocGroup, DocPageId[]])
  .filter(([, pages]) => pages.length);
const docsSearchDocuments = docPages.flatMap((page) => page.searchSections);

function parseDocPage(modulePath: string, raw: string): DocPage {
  const match = raw.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n?/);
  if (!match) throw new Error(`${modulePath} is missing frontmatter`);

  const meta = parseFrontmatter(match[1]);
  const body = raw.slice(match[0].length).trim();
  const blocks = parseMarkdown(body);
  const page: DocMeta = {
    id: requiredMeta(meta, "id", modulePath),
    path: meta.path ?? "",
    label: requiredMeta(meta, "label", modulePath),
    title: requiredMeta(meta, "title", modulePath),
    summary: requiredMeta(meta, "summary", modulePath),
    group: parseTocGroup(requiredMeta(meta, "group", modulePath), modulePath),
    order: Number(requiredMeta(meta, "order", modulePath)),
    updated: requiredMeta(meta, "updated", modulePath),
  };

  return {
    ...page,
    blocks,
    searchSections: buildPageSearchSections(page, blocks),
  };
}

function parseFrontmatter(text: string) {
  const meta: Record<string, string> = {};
  text.split(/\r?\n/).forEach((line) => {
    const separator = line.indexOf(":");
    if (separator === -1) return;
    meta[line.slice(0, separator).trim()] = line.slice(separator + 1).trim();
  });
  return meta;
}

function requiredMeta(meta: Record<string, string>, key: string, modulePath: string) {
  const value = meta[key];
  if (value === undefined) throw new Error(`${modulePath} is missing frontmatter key ${key}`);
  return value;
}

function parseTocGroup(value: string, modulePath: string): TocGroup {
  if (tocOrder.includes(value as TocGroup)) return value as TocGroup;
  throw new Error(`${modulePath} has unknown docs group ${value}`);
}

function parseMarkdown(markdown: string): MarkdownBlock[] {
  const lines = markdown.split(/\r?\n/);
  const blocks: MarkdownBlock[] = [];
  const usedIds = new Map<string, number>();
  let index = 0;

  while (index < lines.length) {
    const line = lines[index];
    if (!line.trim()) {
      index += 1;
      continue;
    }

    const fence = line.match(/^```(\S*)?\s*(.*)$/);
    if (fence) {
      const code: string[] = [];
      index += 1;
      while (index < lines.length && !lines[index].startsWith("```")) {
        code.push(lines[index]);
        index += 1;
      }
      index += index < lines.length ? 1 : 0;
      blocks.push({
        type: "code",
        language: fence[1] ?? "",
        title: fence[2]?.trim() || fence[1] || "Example",
        code: code.join("\n"),
      });
      continue;
    }

    const heading = line.match(/^(#{2,4})\s+(.+)$/);
    if (heading) {
      const text = heading[2].trim();
      blocks.push({
        type: "heading",
        level: heading[1].length,
        id: uniqueSlug(text, usedIds),
        text,
      });
      index += 1;
      continue;
    }

    if (isTableStart(lines, index)) {
      const headers = parseTableRow(lines[index]);
      const rows: string[][] = [];
      index += 2;
      while (index < lines.length && lines[index].trim().startsWith("|")) {
        rows.push(parseTableRow(lines[index]));
        index += 1;
      }
      blocks.push({ type: "table", headers, rows });
      continue;
    }

    if (/^\s*-\s+/.test(line)) {
      const items: string[] = [];
      while (index < lines.length && /^\s*-\s+/.test(lines[index])) {
        items.push(lines[index].replace(/^\s*-\s+/, "").trim());
        index += 1;
      }
      blocks.push({ type: "list", items });
      continue;
    }

    if (/^\s*>\s?/.test(line)) {
      const quote: string[] = [];
      while (index < lines.length && /^\s*>\s?/.test(lines[index])) {
        quote.push(lines[index].replace(/^\s*>\s?/, ""));
        index += 1;
      }
      const [first, ...rest] = quote;
      const titleMatch = first.match(/^\[!(\w+)\]\s*(.*)$/);
      blocks.push({
        type: "quote",
        title: titleMatch ? titleMatch[2] || titleMatch[1] : "Note",
        body: titleMatch ? rest.join(" ") : quote.join(" "),
      });
      continue;
    }

    const paragraph: string[] = [];
    while (index < lines.length && lines[index].trim() && !isBlockStart(lines, index)) {
      paragraph.push(lines[index].trim());
      index += 1;
    }
    blocks.push({ type: "paragraph", text: paragraph.join(" ") });
  }

  return blocks;
}

function isBlockStart(lines: string[], index: number) {
  const line = lines[index];
  return /^```/.test(line)
    || /^(#{2,4})\s+/.test(line)
    || /^\s*-\s+/.test(line)
    || /^\s*>\s?/.test(line)
    || isTableStart(lines, index);
}

function isTableStart(lines: string[], index: number) {
  return Boolean(
    lines[index]?.trim().startsWith("|")
      && lines[index + 1]
      && /^\s*\|?[\s:-]+\|[\s|:-]+\|?\s*$/.test(lines[index + 1]),
  );
}

function parseTableRow(line: string) {
  return line.trim().replace(/^\|/, "").replace(/\|$/, "").split("|").map((cell) => cell.trim());
}

function uniqueSlug(text: string, usedIds: Map<string, number>) {
  const base = stripInline(text).toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "section";
  const count = usedIds.get(base) ?? 0;
  usedIds.set(base, count + 1);
  return count ? `${base}-${count + 1}` : base;
}

function stripInline(text: string) {
  return text
    .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
    .replace(/[`*_]/g, "");
}

function buildPageSearchSections(page: DocMeta, blocks: MarkdownBlock[]) {
  const sections: SearchDoc[] = [];
  let current: SearchDoc = {
    id: `${page.id}:top`,
    pageId: page.id,
    anchor: page.id,
    title: page.title,
    summary: page.summary,
    text: page.summary,
  };

  const pushCurrent = () => {
    const text = compactText(current.text);
    if (!text) return;
    sections.push({
      ...current,
      text,
      summary: current.summary || summarize(text),
    });
  };

  blocks.forEach((block) => {
    if (block.type === "heading") {
      pushCurrent();
      current = {
        id: `${page.id}:${block.id}`,
        pageId: page.id,
        anchor: block.id,
        title: block.text,
        summary: page.summary,
        text: block.text,
      };
      return;
    }
    current.text += ` ${blockSearchText(block)}`;
  });

  pushCurrent();
  return sections;
}

function blockSearchText(block: MarkdownBlock) {
  if (block.type === "paragraph") return block.text;
  if (block.type === "list") return block.items.join(" ");
  if (block.type === "table") return [...block.headers, ...block.rows.flat()].join(" ");
  if (block.type === "code") return `${block.title} ${block.code}`;
  if (block.type === "quote") return `${block.title} ${block.body}`;
  return "";
}

function compactText(text: string) {
  return stripInline(text).replace(/\s+/g, " ").trim();
}

function summarize(text: string) {
  const compact = compactText(text);
  return compact.length > 160 ? `${compact.slice(0, 157)}...` : compact;
}

function tokenize(text: string) {
  return text.toLowerCase().match(/[a-z0-9_./:-]+/g) ?? [];
}

function buildSearchIndex(docs: SearchDoc[]): SearchIndex {
  const indexedDocs = docs.map((doc) => ({
    ...doc,
    tokens: tokenize(`${doc.title} ${doc.summary} ${doc.text}`),
    titleTokens: tokenize(doc.title),
  }));
  const docFreq = new Map<string, number>();

  indexedDocs.forEach((doc) => {
    new Set(doc.tokens).forEach((token) => {
      docFreq.set(token, (docFreq.get(token) ?? 0) + 1);
    });
  });

  return {
    docs: indexedDocs,
    avgLength: indexedDocs.reduce((sum, doc) => sum + doc.tokens.length, 0) / Math.max(indexedDocs.length, 1),
    docFreq,
  };
}

function searchDocs(index: SearchIndex, query: string) {
  const terms = [...new Set(tokenize(query))];
  if (!terms.length) return [];

  const k1 = 1.2;
  const b = 0.75;
  const exactQuery = terms.join(" ");
  return index.docs
    .map((doc) => {
      const counts = new Map<string, number>();
      doc.tokens.forEach((token) => counts.set(token, (counts.get(token) ?? 0) + 1));

      const score = terms.reduce((sum, term) => {
        const tf = counts.get(term) ?? 0;
        if (!tf) return sum;

        const df = index.docFreq.get(term) ?? 0;
        const idf = Math.log(1 + (index.docs.length - df + 0.5) / (df + 0.5));
        const normalizedTf = (tf * (k1 + 1)) / (tf + k1 * (1 - b + b * (doc.tokens.length / index.avgLength)));
        const titleBoost = doc.titleTokens.includes(term) ? 1.8 : 1;
        return sum + idf * normalizedTf * titleBoost;
      }, 0);

      const haystack = `${doc.title} ${doc.summary} ${doc.text}`.toLowerCase();
      return { ...doc, score: score + (haystack.includes(exactQuery) ? 2.4 : 0) };
    })
    .filter((doc) => doc.score > 0)
    .sort((a, b) => b.score - a.score)
    .slice(0, 5);
}

function docsHref(path: string, anchor?: string) {
  const isDocsHost = typeof window === "undefined" || window.location.hostname === "docs.milim.ai";
  const base = isDocsHost ? "" : "/docs";
  const href = path ? `${base}/${path}` : base || "/";
  return anchor ? `${href}#${anchor}` : href;
}

function currentDocPage(pathname?: string) {
  const path = (pathname ?? window.location.pathname).replace(/\/+$/, "") || "/";
  const isDocsHost = typeof window === "undefined" || window.location.hostname === "docs.milim.ai";
  const slug = isDocsHost
    ? path.replace(/^\/(?:docs|wiki)\/?/, "").replace(/^\//, "")
    : (path.match(/^\/(?:docs|wiki)(?:\/(.*))?$/)?.[1] ?? "");

  return docPages.find((page) => page.path === slug) ?? docsPageById.get("overview")!;
}

function formatUpdated(date: string) {
  return new Intl.DateTimeFormat("en", {
    month: "short",
    day: "numeric",
    year: "numeric",
    timeZone: "UTC",
  }).format(new Date(`${date}T00:00:00Z`));
}

export function DocsPage({ pathname }: { pathname?: string } = {}) {
  const root = useRef<HTMLDivElement>(null);
  const currentPage = currentDocPage(pathname);
  const isOverviewPage = currentPage.id === "overview";
  const searchIndex = useMemo(() => buildSearchIndex(docsSearchDocuments), []);
  const [scrollProgress, setScrollProgress] = useState(0);

  useEffect(() => {
    const canonicalPath = currentPage.path ? `/${currentPage.path}` : "/";
    const title = isOverviewPage ? "milim docs - Wiki" : `${currentPage.title} - milim docs`;

    document.title = title;
    document.querySelector('meta[name="description"]')?.setAttribute("content", currentPage.summary);
    document.querySelector('link[rel="canonical"]')?.setAttribute("href", `https://docs.milim.ai${canonicalPath}`);
    document.querySelector('meta[property="og:url"]')?.setAttribute("content", `https://docs.milim.ai${canonicalPath}`);
    document.querySelector('meta[property="og:title"]')?.setAttribute("content", title);
    document.querySelector('meta[property="og:description"]')?.setAttribute("content", currentPage.summary);
    document.querySelector('meta[name="twitter:title"]')?.setAttribute("content", title);
    document.querySelector('meta[name="twitter:description"]')?.setAttribute("content", currentPage.summary);
  }, [currentPage, isOverviewPage]);

  useEffect(() => {
    const legacyPage = docPages.find((page) => page.id === window.location.hash.slice(1));
    if (legacyPage?.path) {
      window.location.replace(docsHref(legacyPage.path));
      return;
    }

    const scrollToHash = () => {
      const id = window.location.hash.slice(1);
      if (!id) return;
      requestAnimationFrame(() => document.getElementById(id)?.scrollIntoView());
    };

    scrollToHash();
    window.addEventListener("hashchange", scrollToHash);
    return () => window.removeEventListener("hashchange", scrollToHash);
  }, []);

  useEffect(() => {
    const updateScrollProgress = () => {
      const scrollable = document.documentElement.scrollHeight - window.innerHeight;
      const progress = scrollable <= 0 ? 1 : window.scrollY / scrollable;
      setScrollProgress(Math.round(Math.min(1, Math.max(0, progress)) * 100));
    };

    updateScrollProgress();
    window.addEventListener("scroll", updateScrollProgress, { passive: true });
    window.addEventListener("resize", updateScrollProgress);
    return () => {
      window.removeEventListener("scroll", updateScrollProgress);
      window.removeEventListener("resize", updateScrollProgress);
    };
  }, []);

  useGSAP(
    () => {
      if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;

      const heroCopyItems = gsap.utils.toArray<HTMLElement>(".docs-hero-copy > *");
      if (heroCopyItems.length) {
        gsap.from(heroCopyItems, {
          y: 24,
          opacity: 0,
          duration: 0.8,
          stagger: 0.07,
          ease: "power3.out",
        });
      }

      const snapshotFeatures = gsap.utils.toArray<HTMLElement>(".docs-snapshot-feature");
      if (snapshotFeatures.length) {
        gsap.from(snapshotFeatures, {
          x: 18,
          opacity: 0,
          duration: 0.7,
          ease: "power3.out",
        });
      }

      gsap.utils.toArray<HTMLElement>(".doc-section").forEach((section) => {
        const sectionItems = section.querySelectorAll(".doc-example, .doc-table-wrap");
        if (sectionItems.length) {
          gsap.from(sectionItems, {
            opacity: 0,
            duration: 0.62,
            stagger: 0.035,
            ease: "power3.out",
            scrollTrigger: {
              trigger: section,
              start: "top 74%",
            },
          });
        }
      });

      const desktop = gsap.matchMedia();
      desktop.add("(min-width: 981px)", () => {
        ScrollTrigger.create({
          trigger: ".docs-layout",
          pin: ".docs-toc-shell",
          start: "top 88px",
          end: "bottom bottom",
          pinSpacing: false,
        });

        gsap.utils.toArray<HTMLElement>(".doc-section").forEach((section) => {
          gsap.fromTo(
            section,
            { opacity: 0.7, scale: 0.985 },
            {
              opacity: 1,
              scale: 1,
              scrollTrigger: {
                trigger: section,
                start: "top 86%",
                end: "top 48%",
                scrub: true,
              },
            },
          );
        });
      });

      return () => desktop.revert();
    },
    { scope: root },
  );

  return (
    <div ref={root} className="site-shell docs-shell">
      <a className="skip-link" href="#docs-main-content">Skip to content</a>
      <header className="nav">
        <a className="brand" href="/" aria-label="milim home">
          <img src="/assets/milim-wordmark.svg" alt="" />
        </a>
        <div className="docs-nav-tools">
          <a className="docs-nav-title" href={docsHref("")}>docs</a>
          <DocsSearch index={searchIndex} variant="nav" />
        </div>
        <div className="nav-actions">
          <nav aria-label="Primary">
            {docsNavLinks.map((link) => <a href={link.href} key={link.href}>{link.label}</a>)}
          </nav>
          <ThemeControl />
          <SiteMobileNav links={docsNavLinks} />
        </div>
      </header>

      <main id="docs-main-content" className={`docs-main${isOverviewPage ? "" : " docs-main-subpage"}`} tabIndex={-1}>
        {isOverviewPage ? (
          <section className="docs-hero" id="top">
            <ShaderField />
            <HeroAsciiField />
            <div className="docs-hero-inner">
              <div className="docs-hero-copy">
                <Eyebrow index={`v${DOCS_VERSION}`} label={`updated ${formatUpdated(currentPage.updated)}`} />
                <h1>
                  milim{" "}
                  <br />
                  docs wiki
                </h1>
                <p>{currentPage.summary}</p>
                <div className="docs-actions">
                  <a className="button button-primary" href={WINDOWS_URL}>Download Windows</a>
                  <a className="button button-secondary" href={MACOS_URL}>Download macOS</a>
                  <a className="source-link" href={GITHUB_URL}>Source</a>
                </div>
              </div>
              <nav className="docs-snapshot" aria-label="Docs map">
              <a className="docs-snapshot-feature" href={docsHref("quickstart")}>
                <Eyebrow label="start with" />
                <strong>quickstart</strong>
                <em>connect a model, pick a folder, send a useful prompt</em>
              </a>
              <div className="docs-snapshot-paths">
                <a href={docsHref("desktop")}>
                  <Eyebrow label="operate" />
                  <strong>desktop</strong>
                </a>
                <a href={docsHref("models")}>
                  <Eyebrow label="route" />
                  <strong>models</strong>
                </a>
                <a href={docsHref("privacy")}>
                  <Eyebrow label="contain" />
                  <strong>privacy</strong>
                </a>
                <a href={docsHref("api")}>
                  <Eyebrow label="build" />
                  <strong>api</strong>
                </a>
              </div>
              </nav>
            </div>
          </section>
        ) : (
          <section className="docs-page-head">
            <a href={docsHref("")}>docs</a>
            <Eyebrow index={`v${DOCS_VERSION}`} label={`updated ${formatUpdated(currentPage.updated)}`} />
            <h1>{currentPage.title}</h1>
            <p>{currentPage.summary}</p>
          </section>
        )}

        <div className="docs-layout">
          <div className="docs-toc-shell">
            <aside className="docs-scroll-rail" aria-hidden="true" style={{ "--docs-scroll-progress": `${scrollProgress}%` } as CSSProperties}>
              <span className="docs-scroll-track">
                <span className="docs-scroll-fill" />
              </span>
              <span className="docs-scroll-percent">{scrollProgress}%</span>
            </aside>
            <aside className="docs-toc" aria-label="Docs table of contents">
              <strong>Contents</strong>
              {tocGroups.map(([group, items]) => (
                <div className="docs-toc-group" key={group}>
                  <Eyebrow label={group} />
                  {items.map((id) => {
                    const page = docsPageById.get(id)!;
                    return (
                      <a href={docsHref(page.path)} aria-current={currentPage.id === page.id ? "page" : undefined} key={id}>
                        {page.label}
                      </a>
                    );
                  })}
                </div>
              ))}
            </aside>
          </div>

          <div className="docs-content">
            <article className="doc-section" id={currentPage.id}>
              <MarkdownBlocks blocks={currentPage.blocks} />
            </article>
          </div>
        </div>
      </main>
    </div>
  );
}

function DocsSearch({ index, variant }: { index: SearchIndex; variant?: "nav" }) {
  const [query, setQuery] = useState("");
  const trimmedQuery = query.trim();
  const results = useMemo(
    () => trimmedQuery.length < 2 ? [] : searchDocs(index, trimmedQuery),
    [index, trimmedQuery],
  );

  return (
    <div className={`docs-search${variant === "nav" ? " docs-search-nav" : ""}`}>
      <label htmlFor="docs-search">Search</label>
      <input
        id="docs-search"
        type="search"
        value={query}
        onChange={(event) => setQuery(event.currentTarget.value)}
        placeholder="models, privacy, api..."
      />
      {trimmedQuery.length >= 2 ? (
        <div className="docs-search-results" data-lenis-prevent>
          <div className="docs-search-results-head">
            <span>{results.length ? "Top matches" : "No matches"}</span>
            {results.length ? <em>{results.length}</em> : null}
          </div>
          {results.length ? results.map((result) => {
            const page = docsPageById.get(result.pageId)!;
            return (
              <a href={docsHref(page.path, result.anchor)} key={result.id}>
                <Eyebrow label={`docs / ${page.label}`} />
                <strong>{result.title}</strong>
                <span>{result.summary}</span>
              </a>
            );
          }) : <p>Try models, privacy, agents, memory, MCP, or msk-v1.</p>}
        </div>
      ) : null}
    </div>
  );
}

function MarkdownBlocks({ blocks }: { blocks: MarkdownBlock[] }) {
  return (
    <>
      {blocks.map((block, index) => {
        if (block.type === "heading") return <MarkdownHeading block={block} key={`${block.id}-${index}`} />;
        if (block.type === "paragraph") return <p key={index}><InlineMarkdown text={block.text} /></p>;
        if (block.type === "list") {
          return (
            <ul className="doc-list" key={index}>
              {block.items.map((item) => <li key={item}><InlineMarkdown text={item} /></li>)}
            </ul>
          );
        }
        if (block.type === "table") return <Table headers={block.headers} rows={block.rows} key={index} />;
        if (block.type === "code") return <Example title={block.title} language={block.language} key={index}>{block.code}</Example>;
        return (
          <aside className="doc-callout" key={index}>
            <strong>{block.title}</strong>
            <p><InlineMarkdown text={block.body} /></p>
          </aside>
        );
      })}
    </>
  );
}

function MarkdownHeading({ block }: { block: Extract<MarkdownBlock, { type: "heading" }> }) {
  const content = (
    <>
      <a className="doc-anchor" href={`#${block.id}`} aria-label={`Link to ${block.text}`}>#</a>
      <InlineMarkdown text={block.text} />
    </>
  );
  if (block.level === 2) return <h2 id={block.id}>{content}</h2>;
  if (block.level === 3) return <h3 id={block.id}>{content}</h3>;
  return <h4 id={block.id}>{content}</h4>;
}

function Table({ headers, rows }: { headers: string[]; rows: string[][] }) {
  return (
    <div className="doc-table-wrap">
      <table className={`doc-table doc-table-${headers.length}`}>
        <thead>
          <tr>{headers.map((header) => <th key={header}>{header}</th>)}</tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row.join("|")}>
              {row.map((cell, index) => (
                <td key={`${cell}-${index}`}>
                  <InlineMarkdown text={cell} />
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function InlineMarkdown({ text }: { text: string }) {
  return <>{inlineNodes(text)}</>;
}

function inlineNodes(text: string): ReactNode[] {
  const nodes: ReactNode[] = [];
  const tokenPattern = /(`[^`]+`|\*\*[^*]+\*\*|\[[^\]]+\]\([^)]+\))/g;
  let lastIndex = 0;
  let match: RegExpExecArray | null;

  while ((match = tokenPattern.exec(text)) !== null) {
    if (match.index > lastIndex) nodes.push(text.slice(lastIndex, match.index));
    const token = match[0];
    if (token.startsWith("`")) {
      nodes.push(<code key={nodes.length}>{token.slice(1, -1)}</code>);
    } else if (token.startsWith("**")) {
      nodes.push(<strong key={nodes.length}>{token.slice(2, -2)}</strong>);
    } else {
      const link = token.match(/^\[([^\]]+)\]\(([^)]+)\)$/);
      if (link) nodes.push(<a href={link[2]} key={nodes.length}>{link[1]}</a>);
    }
    lastIndex = tokenPattern.lastIndex;
  }

  if (lastIndex < text.length) nodes.push(text.slice(lastIndex));
  return nodes;
}

function Example({ title, language, children }: { title: string; language: string; children: string }) {
  const [copied, setCopied] = useState(false);
  const resetTimer = useRef<number>();

  useEffect(() => () => window.clearTimeout(resetTimer.current), []);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(children.trim());
      window.clearTimeout(resetTimer.current);
      setCopied(true);
      resetTimer.current = window.setTimeout(() => setCopied(false), 1200);
    } catch {
      // Clipboard access can be blocked; leave the button ready to retry.
    }
  };

  return (
    <figure className="doc-example">
      <figcaption>
        <span>{title}</span>
        <button
          className="doc-copy"
          type="button"
          aria-label={copied ? "Copied" : "Copy code"}
          data-copied={copied ? "" : undefined}
          onClick={() => void copy()}
        >
          <span className="doc-copy-labels" aria-hidden="true">
            <span>Copy</span>
            <span>Copied</span>
          </span>
          <span className="doc-copy-status" aria-live="polite">{copied ? "Code copied" : ""}</span>
        </button>
      </figcaption>
      <pre data-lenis-prevent><code className={language ? `language-${language}` : undefined}>{children.trim()}</code></pre>
    </figure>
  );
}
