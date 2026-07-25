import { lazy, Suspense, useEffect, useRef, useState, type ReactNode, type SVGProps } from "react";
import { useGSAP } from "@gsap/react";
import gsap from "gsap";
import { ScrollTrigger } from "gsap/ScrollTrigger";
import Lenis from "lenis";
import { Eyebrow } from "./Eyebrow";
import { HeroAsciiField } from "./HeroAsciiField";
import { ShaderField } from "./ShaderField";
import { SiteMobileNav } from "./SiteMobileNav";
import { ThemeControl } from "./ThemeControl";

gsap.registerPlugin(useGSAP, ScrollTrigger);

const WINDOWS_URL = "https://github.com/oshtz/milim/releases/latest/download/milim-windows-x64-portable.exe";
const MACOS_URL = "https://github.com/oshtz/milim/releases/latest/download/milim-macos-universal.dmg";
const GITHUB_URL = "https://github.com/oshtz/milim";
const RELEASES_URL = "https://github.com/oshtz/milim/releases/latest";
const DOCS_URL = "https://docs.milim.ai/";
const DOCS_QUICKSTART_URL = `${DOCS_URL}quickstart`;
const GITHUB_RELEASE_API_URL = "https://api.github.com/repos/oshtz/milim/releases/latest";
const GITHUB_REPO_API_URL = "https://api.github.com/repos/oshtz/milim";
const RELEASE_CACHE_KEY = "milim-release-latest";
const DocsPage = lazy(() => import("./DocsPage").then(({ DocsPage: Page }) => ({ default: Page })));

type DownloadPlatform = "windows" | "macos";

type GitHubAsset = {
  name?: string;
  browser_download_url?: string;
  size?: number;
};

type GitHubRelease = {
  tag_name?: string;
  html_url?: string;
  assets?: GitHubAsset[];
};

type GitHubRepo = {
  stargazers_count?: number;
};

type ReleaseDownload = {
  href: string;
  sizeLabel?: string;
};

type ReleaseDownloads = {
  tagName?: string;
  releaseUrl: string;
  windows: ReleaseDownload;
  macos: ReleaseDownload;
};

const fallbackDownloads: ReleaseDownloads = {
  releaseUrl: RELEASES_URL,
  windows: { href: WINDOWS_URL },
  macos: { href: MACOS_URL },
};

const navLinks = [
  { label: "Docs", href: DOCS_URL, className: "nav-docs-link" },
  { label: "Product", href: "/#product" },
  { label: "How it works", href: "/#workflow" },
  { label: "Quickstart", href: "/#quickstart" },
  { label: "Download", href: "/#releases" },
  { label: "GitHub", href: GITHUB_URL },
];

const quickstartSteps = [
  {
    step: "download",
    title: "Install the app",
    body: "Grab the Windows portable executable or macOS universal disk image from GitHub Releases.",
  },
  {
    step: "add a key",
    title: "Connect a model source",
    body: "Add a provider key, sign in to an account runtime, or point milim at Ollama or LM Studio.",
  },
  {
    step: "develop",
    title: "Start a dev thread",
    body: "Pick a workspace, ask for an edit or test run, switch models, and keep the same project context.",
  },
];

const faqItems = [
  {
    id: "faq-machine-boundary",
    question: "Does anything leave my machine?",
    answer:
      "Local runtimes stay on loopback. Hosted providers are called only when you choose them, and remote requests can pass through redact or block mode first.",
  },
  {
    id: "faq-providers",
    question: "Which providers?",
    answer:
      "OpenAI-compatible endpoints, OpenAI, OpenRouter, Groq, Anthropic, Gemini, Replicate, fal, Ollama, LM Studio, Codex, and Claude Code are covered.",
  },
  {
    id: "faq-linux",
    question: "Is Linux supported?",
    answer:
      "Windows and macOS are the release artifacts. Linux packaging is disabled for now, but the Rust server and Tauri app can still be built from source.",
  },
  {
    id: "faq-free",
    question: "Is it really free?",
    answer:
      "Yes. The repo is MIT licensed. Provider usage depends on the keys, accounts, or local runtimes you connect.",
  },
  {
    id: "faq-google-workspace",
    question: "Why does Milim connect to Google Drive?",
    answer:
      "Only to let you choose, view, and work with specific Drive, Docs, Sheets, and Slides files inside Milim. It requests per-file access rather than access to your whole Drive; details and removal controls are in the privacy policy.",
  },
  {
    id: "faq-difference",
    question: "How is Milim different from Claude Code, LM Studio, or Open WebUI?",
    answer:
      "Those tools are strong at a specific runtime or interface. Milim is the local control plane around them: one project thread, visible tool runs, review controls, local memory, and provider switching without rebuilding the work around one vendor.",
  },
];

const features = [
  {
    title: "One desktop app, not a toolchain",
    body: "Model switching, generated artifacts, previews, voice, schedules, themes, and provider setup sit in a single cross-platform shell instead of spreading across terminals and browser tabs.",
    wide: true,
  },
  {
    id: "review",
    title: "Nothing lands without approval",
    body: "Edits and tool calls arrive as proposals with a readable diff. The agent suggests; you decide what actually touches the repo.",
    visual: true,
  },
  {
    title: "Works on your actual repo",
    body: "Git status, diffs, branches, and pull requests sit beside the thread, so the agent operates on the project you already have instead of a sandbox copy of it.",
    full: true,
  },
];

type ChapterKind = "models" | "privacy" | "tools" | "memory";

const chapters: Array<{ id?: string; title: string; kicker: string; body: string; kind: ChapterKind }> = [
  {
    title: "Model freedom",
    kicker: "routing",
    body: "Switch between hosted APIs, local runtimes, account runtimes, and media models without rebuilding the thread around one vendor.",
    kind: "models",
  },
  {
    id: "privacy",
    title: "Privacy control",
    kicker: "boundary",
    body: "Keep local model traffic untouched and gate remote traffic with deterministic redaction or blocking.",
    kind: "privacy",
  },
  {
    title: "Agents and tools",
    kicker: "execution",
    body: "Run tools with visible timelines: each model step, tool call, result, error, and elapsed time stays inspectable.",
    kind: "tools",
  },
  {
    title: "Local memory",
    kicker: "context",
    body: "Ingest project context, search it semantically, and keep the useful parts near the thread.",
    kind: "memory",
  },
];

const chapterVisuals: Record<ChapterKind, { label: string; foot: string }> = {
  models: { label: "model router", foot: "one thread / four sources" },
  privacy: { label: "outbound gate", foot: "local passthrough / remote redacted" },
  tools: { label: "tool run", foot: "3 steps / 1.9s" },
  memory: { label: "local recall", foot: "3 chunks pinned to thread" },
};

const routeSources = [
  { name: "openai", tag: "hosted" },
  { name: "ollama", tag: "local" },
  { name: "claude code", tag: "account" },
  { name: "custom /v1", tag: "compatible" },
];

const toolSteps = [
  { name: "mcp:list_files", meta: "42ms", detail: "src/App.tsx" },
  { name: "sandbox:run", meta: "1.8s", detail: "pnpm build" },
  { name: "diff:review", meta: "ready", detail: "3 checks passed" },
];

const recallHits = [
  { path: "auth/session.rs", score: "0.92" },
  { path: "docs/wiki/auth.md", score: "0.81" },
  { path: "api/tokens.ts", score: "0.64" },
];

export function App() {
  useEffect(() => {
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;

    const lenis = new Lenis({ autoRaf: true, anchors: true, stopInertiaOnNavigate: true });
    lenis.on("scroll", ScrollTrigger.update);
    return () => lenis.destroy();
  }, []);

  const path = window.location.pathname.replace(/\/+$/, "") || "/";
  if (window.location.hostname === "docs.milim.ai") return <Suspense fallback={<DocsFallback />}><DocsPage /></Suspense>;
  if (path === "/docs" || path === "/wiki" || path.startsWith("/docs/") || path.startsWith("/wiki/")) {
    return <Suspense fallback={<DocsFallback />}><DocsPage /></Suspense>;
  }
  return <LandingPage />;
}

export function LandingPage() {
  const root = useRef<HTMLDivElement>(null);
  const [downloadPlatform, setDownloadPlatform] = useState<DownloadPlatform | null>(null);
  const [downloads, setDownloads] = useState<ReleaseDownloads>(fallbackDownloads);
  const [githubStars, setGithubStars] = useState<number | null>(null);

  useEffect(() => {
    setDownloadPlatform(detectDownloadPlatform());
  }, []);

  useEffect(() => {
    let cancelled = false;
    fetch(GITHUB_REPO_API_URL, { headers: { Accept: "application/vnd.github+json" } })
      .then((response) => (response.ok ? response.json() : Promise.reject()))
      .then((repo: GitHubRepo) => {
        if (!cancelled && Number.isFinite(repo.stargazers_count)) setGithubStars(repo.stargazers_count!);
      })
      .catch(() => {});

    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;

    const cached = readCachedRelease();
    if (cached) {
      setDownloads(downloadsFromRelease(cached));
      return;
    }

    fetch(GITHUB_RELEASE_API_URL, { headers: { Accept: "application/vnd.github+json" } })
      .then((response) => (response.ok ? response.json() : Promise.reject()))
      .then((release: GitHubRelease) => {
        if (cancelled) return;
        cacheRelease(release);
        setDownloads(downloadsFromRelease(release));
      })
      .catch(() => {
        if (!cancelled) setDownloads(fallbackDownloads);
      });

    return () => {
      cancelled = true;
    };
  }, []);

  useGSAP(
    () => {
      const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
      if (reduceMotion) return;

      gsap.from(".hero-media", {
        y: 38,
        opacity: 0,
        scale: 0.94,
        duration: 1.1,
        ease: "power3.out",
      });

      gsap.to(".hero-media", {
        y: -72,
        scale: 0.97,
        scrollTrigger: {
          trigger: ".hero",
          start: "top top",
          end: "bottom top",
          scrub: true,
        },
      });

      gsap.utils.toArray<HTMLElement>(".reveal").forEach((element) => {
        gsap.from(element, {
          y: 42,
          opacity: 0,
          duration: 0.8,
          ease: "power3.out",
          scrollTrigger: {
            trigger: element,
            start: "top 78%",
          },
        });
      });

      gsap.from(".feature-cell", {
        y: 22,
        opacity: 0,
        duration: 0.7,
        stagger: 0.06,
        ease: "power3.out",
        scrollTrigger: {
          trigger: ".feature-grid",
          start: "top 78%",
        },
      });

      const streamLines = gsap.utils.toArray<HTMLElement>(".mini-stream-text span");
      const composerText = document.querySelector<HTMLElement>(".mini-composer-input span");
      const streamTimeline = gsap.timeline({
        paused: true,
        repeat: -1,
        repeatDelay: 0.8,
      })
        .set(".mini-sent-message, .mini-response, .mini-diff-card, .mini-model-switch-note", {
          autoAlpha: 0,
          y: 12,
        })
        .set(".mini-composer-card", { y: -112 })
        .set(".mini-composer-input span, .mini-stream-text span", {
          clipPath: "inset(0 100% 0 0)",
          opacity: 0,
        })
        .set(".mini-model-a", { autoAlpha: 1 })
        .set(".mini-model-b", { autoAlpha: 0 })
        .set(".mini-tool-row span", { y: 5, opacity: 0.48 })
        .to(".mini-composer-input span", {
          clipPath: "inset(0 0% 0 0)",
          opacity: 1,
          duration: 1.35,
          ease: `steps(${composerText?.textContent?.length ?? 46})`,
        })
        .to(".mini-composer-send b", {
          y: -1,
          scale: 1.04,
          duration: 0.16,
          ease: "power2.out",
        })
        .to(".mini-composer-send b", { y: 0, scale: 1, duration: 0.2 })
        .to(".mini-composer-card", { y: 0, duration: 0.72, ease: "power3.inOut" }, "<")
        .to(".mini-sent-message", { autoAlpha: 1, y: 0, duration: 0.4, ease: "power2.out" }, "-=0.25")
        .to(".mini-response", { autoAlpha: 1, y: 0, duration: 0.42, ease: "power2.out" })
        .to(".mini-stream-event i", {
          scale: 1.28,
          duration: 0.22,
          boxShadow: "0 0 24px rgba(184, 195, 165, 0.48)",
          ease: "power2.out",
        })
        .to(".mini-stream-event i", {
          scale: 1,
          boxShadow: "0 0 18px rgba(184, 195, 165, 0.28)",
          duration: 0.32,
          ease: "power2.inOut",
        });

      streamLines.forEach((line) => {
        streamTimeline.to(line, {
          clipPath: "inset(0 0% 0 0)",
          opacity: 1,
          duration: Math.max(0.7, (line.textContent?.length ?? 30) / 32),
          ease: `steps(${line.textContent?.length ?? 30})`,
        });
      });

      streamTimeline
        .to(".mini-tool-row span", {
          y: 0,
          opacity: 1,
          duration: 0.34,
          stagger: 0.09,
          ease: "power2.out",
        }, "<")
        .to(".mini-model-a", { autoAlpha: 0, duration: 0.24 }, "+=0.35")
        .to(".mini-model-b", { autoAlpha: 1, duration: 0.24 }, "<")
        .to(".mini-model-switch-note", { autoAlpha: 1, y: 0, duration: 0.3 }, "<")
        .to(".mini-diff-card", { autoAlpha: 1, y: 0, duration: 0.48, ease: "power2.out" }, "+=0.18")
        .to(".mini-review-approve", {
          borderColor: "rgba(184, 195, 165, 0.58)",
          color: "var(--object-text)",
          duration: 0.28,
        }, "+=0.35")
        .to({}, { duration: 1.15 })
        .to(".mini-sent-message, .mini-response, .mini-diff-card", {
          autoAlpha: 0,
          y: -8,
          duration: 0.45,
          ease: "power2.in",
        })
        .to(".mini-composer-card", { y: -112, duration: 0.5, ease: "power2.inOut" }, "<");
      ScrollTrigger.create({
        trigger: ".hero-media",
        start: "top bottom",
        end: "bottom top",
        onToggle: ({ isActive }) => isActive ? streamTimeline.play() : streamTimeline.pause(),
      });

      gsap.from(".chapter", {
        y: 28,
        opacity: 0,
        duration: 0.7,
        stagger: 0.08,
        ease: "power3.out",
        scrollTrigger: {
          trigger: ".chapter-stack",
          start: "top 78%",
        },
      });

      gsap.utils.toArray<HTMLElement>(".chapter").forEach((chapter) => {
        const trigger = ScrollTrigger.create({
          trigger: chapter,
          start: "top 94%",
          end: "bottom 6%",
          onToggle: ({ isActive }) => chapter.classList.toggle("is-idle", !isActive),
        });
        chapter.classList.toggle("is-idle", !trigger.isActive);
      });
    },
    { scope: root },
  );

  return (
    <div ref={root} className="site-shell">
      <a className="skip-link" href="#main-content">Skip to content</a>
      <Nav />
      <main id="main-content" tabIndex={-1}>
        <section className="hero" id="top">
          <HeroBackgroundEffect />
          <div className="hero-copy">
            <p className="hero-kicker">local-first / model-agnostic / MIT licensed</p>
            <h1>Your local control plane for coding agents.</h1>
            <p className="hero-subline">
              milim lets you use your own models and subscriptions, keep one canonical thread, review the diff, and ship.
              <br className="desktop-copy-break" /> One desktop app.{" "}
              <a className="copy-doc-link" href={DOCS_URL}>Read the docs</a>.
            </p>
            <div className="hero-actions" aria-label="Download milim">
              <DownloadActions downloads={downloads} platform={downloadPlatform} context="hero" />
              <a className="source-link" href={GITHUB_URL}>
                View source <ArrowIcon />
              </a>
            </div>
          </div>
          <WorkbenchObject />
        </section>

        <section className="feature-section reveal" id="product">
          <div className="section-head">
            <Eyebrow label="product" />
            <h2>
              one app.
              <br />
              no black boxes.
            </h2>
            <p>
              A single desktop shell over the whole loop, where every run, diff, and outbound request is something you can open and read.
            </p>
          </div>
          <div className="feature-grid">
            {features.map((feature) => (
              <article
                className={`feature-cell${feature.wide ? " feature-cell-wide" : ""}${feature.visual ? " feature-cell-visual" : ""}${feature.full ? " feature-cell-full" : ""}`}
                id={feature.id}
                key={feature.title}
              >
                <h3>{feature.title}</h3>
                <p>{feature.body}</p>
                {feature.visual ? <ReviewGlyph /> : null}
              </article>
            ))}
          </div>
        </section>

        <section className="story reveal" id="workflow">
          <div className="story-copy">
            <Eyebrow label="how it works" />
            <h2>Switch the model without losing the work.</h2>
            <p>
              The desktop app keeps workspace context, memory, previews, artifacts, approvals, and remote boundaries visible.
            </p>
          </div>
          <div className="chapter-stack">
            {chapters.map((chapter, index) => (
              <article className={`chapter chapter-${chapter.kind}`} id={chapter.id} key={chapter.title}>
                <Eyebrow index={String(index + 1).padStart(2, "0")} label={chapter.kicker} />
                <div className="chapter-copy">
                  <h3>{chapter.title}</h3>
                  <p>{chapter.body}</p>
                </div>
                <ChapterVisual kind={chapter.kind} />
              </article>
            ))}
          </div>
        </section>

        <section className="quickstart-strip reveal" id="quickstart" aria-labelledby="quickstart-title">
          <div>
            <Eyebrow label="quickstart" />
            <h2 id="quickstart-title">download, connect, develop.</h2>
          </div>
          <div className="quickstart-steps">
            {quickstartSteps.map((item, index) => (
              <article className="quickstart-step" key={item.step}>
                <Eyebrow index={String(index + 1).padStart(2, "0")} label={item.step} />
                <h3>{item.title}</h3>
                <p>{item.body}</p>
              </article>
            ))}
          </div>
          <a className="source-link quickstart-link" href={DOCS_QUICKSTART_URL}>
            Read quickstart <ArrowIcon />
          </a>
        </section>

        <section className="download-section reveal" id="releases">
          <div className="download-copy">
            <Eyebrow label="download" />
            <h2 aria-label="Open source. Native desktop.">
              <span>Open source.</span>
              <span>Native desktop.</span>
            </h2>
            <p>
              <strong>Yours to inspect.</strong>{" "}
              Download the Windows portable executable or macOS universal disk image from the latest GitHub release.
              Linux packaging is not a primary release artifact yet; the Rust server and Tauri app remain
              source-buildable; <a className="copy-doc-link" href={DOCS_URL}>docs cover setup</a>.
            </p>
            <p className="release-meta">
              Latest release: <a href={downloads.releaseUrl}>{downloads.tagName ?? "GitHub latest"}</a>
              {githubStars !== null ? ` / ${githubStars.toLocaleString()} GitHub stars` : ""}
            </p>
            <div className="download-actions">
              <DownloadActions downloads={downloads} platform={downloadPlatform} context="release" />
            </div>
          </div>
          <ReleaseObject />
        </section>

        <section className="faq-section reveal" id="faq" aria-labelledby="faq-title">
          <div className="section-head faq-head">
            <Eyebrow label="faq" />
            <h2 id="faq-title">questions before install.</h2>
            <p>Short answers for the local-first and provider-boundary parts people usually check first.</p>
          </div>
          <div className="faq-grid">
            {faqItems.map((item) => (
              <article className="faq-item" id={item.id} key={item.id}>
                <h3>
                  <a href={`#${item.id}`}>{item.question}</a>
                </h3>
                <p>{item.answer}</p>
              </article>
            ))}
          </div>
        </section>

      </main>
      <Footer />
      <FaqJsonLd />
      <SoftwareApplicationJsonLd version={downloads.tagName} />
    </div>
  );
}

function DocsFallback() {
  return (
    <div className="docs-loading" role="status" aria-label="Loading documentation">
      <img src="/assets/milim-wordmark.svg" alt="" />
      <div>
        <span />
        <span />
        <span />
      </div>
    </div>
  );
}

function DownloadActions({
  downloads,
  platform,
  context,
}: {
  downloads: ReleaseDownloads;
  platform: DownloadPlatform | null;
  context: "hero" | "release";
}) {
  const items: Array<{ platform: DownloadPlatform; name: string; label: string; download: ReleaseDownload }> = [
    {
      platform: "windows",
      name: "Windows",
      label: context === "hero" ? "Download for Windows" : "Windows portable EXE",
      download: downloads.windows,
    },
    {
      platform: "macos",
      name: "macOS",
      label: context === "hero" ? "Download for macOS" : "macOS universal DMG",
      download: downloads.macos,
    },
  ];
  const primaryPlatform = platform ?? "windows";

  return (
    <>
      {items.map((item) => {
        const isPrimary = item.platform === primaryPlatform;
        const label = platform && !isPrimary ? `Also available for ${item.name}` : item.label;
        const details = [downloads.tagName, item.download.sizeLabel].filter(Boolean).join(" / ");

        return (
          <a
            className={`button download-button ${isPrimary ? "button-primary" : "button-secondary download-button-alt"}`}
            href={item.download.href}
            key={item.platform}
          >
            <DownloadIcon />
            <span>
              {label}
              {context === "release" && details ? <small>{details}</small> : null}
            </span>
          </a>
        );
      })}
    </>
  );
}

function FaqJsonLd() {
  const json = {
    "@context": "https://schema.org",
    "@type": "FAQPage",
    mainEntity: faqItems.map((item) => ({
      "@type": "Question",
      name: item.question,
      acceptedAnswer: {
        "@type": "Answer",
        text: item.answer,
      },
    })),
  };

  return <script type="application/ld+json" dangerouslySetInnerHTML={{ __html: JSON.stringify(json) }} />;
}

function SoftwareApplicationJsonLd({ version }: { version?: string }) {
  const json = {
    "@context": "https://schema.org",
    "@type": "SoftwareApplication",
    name: "milim",
    description: "A local-first desktop control plane for coding agents, models, tools, memory, and review.",
    applicationCategory: "DeveloperApplication",
    operatingSystem: "Windows, macOS",
    softwareVersion: version,
    offers: {
      "@type": "Offer",
      price: "0",
      priceCurrency: "USD",
    },
  };

  return <script type="application/ld+json" dangerouslySetInnerHTML={{ __html: JSON.stringify(json) }} />;
}

function detectDownloadPlatform(): DownloadPlatform | null {
  const navigatorWithUaData = navigator as Navigator & { userAgentData?: { platform?: string } };
  const platform = `${navigatorWithUaData.userAgentData?.platform ?? navigator.platform} ${navigator.userAgent}`.toLowerCase();
  if (platform.includes("mac")) return "macos";
  if (platform.includes("win")) return "windows";
  return null;
}

function readCachedRelease(): GitHubRelease | null {
  try {
    const cached = sessionStorage.getItem(RELEASE_CACHE_KEY);
    return cached ? (JSON.parse(cached) as GitHubRelease) : null;
  } catch {
    return null;
  }
}

function cacheRelease(release: GitHubRelease) {
  try {
    sessionStorage.setItem(RELEASE_CACHE_KEY, JSON.stringify(release));
  } catch {
    // Storage can be blocked; static latest-release URLs still work.
  }
}

function downloadsFromRelease(release: GitHubRelease): ReleaseDownloads {
  const windows = release.assets?.find((asset) => asset.name === "milim-windows-x64-portable.exe");
  const macos = release.assets?.find((asset) => asset.name === "milim-macos-universal.dmg");

  return {
    releaseUrl: release.html_url || RELEASES_URL,
    tagName: release.tag_name,
    windows: {
      href: windows?.browser_download_url || WINDOWS_URL,
      sizeLabel: formatBytes(windows?.size),
    },
    macos: {
      href: macos?.browser_download_url || MACOS_URL,
      sizeLabel: formatBytes(macos?.size),
    },
  };
}

function formatBytes(size?: number) {
  if (!size || !Number.isFinite(size)) return undefined;
  const megabytes = size / 1024 / 1024;
  return `${megabytes >= 10 ? Math.round(megabytes) : megabytes.toFixed(1)} MB`;
}

function ReviewGlyph() {
  return (
    <div className="chapter-visual feature-glyph" aria-label="Proposed edits waiting on approval" role="img">
      <div className="chapter-visual-bar">
        <span>awaiting approval</span>
        <i />
      </div>
      <div className="chapter-visual-body">
        <ul className="review-files">
          <li className="review-edit"><b>M</b><s>auth/session.rs</s><em>+18 −4</em></li>
          <li className="review-add"><b>A</b><s>auth/rotate.rs</s><em>+61</em></li>
          <li className="review-del"><b>D</b><s>auth/legacy.rs</s><em>−42</em></li>
        </ul>
      </div>
      <p className="chapter-visual-foot">3 files staged / none written yet</p>
    </div>
  );
}

function WorkbenchObject() {
  return (
    <div className="hero-media workbench-object" aria-label="milim desktop workbench concept" role="img">
      <MiniAppShell />
    </div>
  );
}

function MiniAppShell() {
  return (
    <div className="mini-app-shell">
      <aside className="mini-rail" aria-hidden="true">
        <span><MiniSidebarIcon size={16} /></span>
        <span><MiniPlusIcon size={16} /></span>
        <span><MiniSearchIcon size={15} /></span>
        <span className="rail-spacer" />
        <span><MiniCalendarIcon size={15} /></span>
        <span><MiniLightbulbIcon size={15} /></span>
        <span><MiniGearIcon size={15} /></span>
      </aside>

      <div className="mini-content">
        <div className="mini-topbar">
          <div className="mini-topbar-main">
            <span className="mini-wordmark">milim</span>
            <i />
            <strong>New chat</strong>
            <code>reviewing workspace</code>
          </div>
          <div className="mini-topbar-actions" aria-hidden="true">
            <span><MiniPinIcon size={13} /></span>
            <span><MiniWindowMinIcon /></span>
            <span><MiniWindowMaxIcon /></span>
            <span><MiniWindowCloseIcon /></span>
          </div>
        </div>
        <section className="mini-stage">
          <div className="mini-sent-message">
            <p>Run tests, fix the failure, then switch models for review.</p>
          </div>
          <div className="mini-response">
            <div className="mini-stream-event">
              <i />
              <span>tool run</span>
              <code>pnpm test</code>
            </div>
            <p className="mini-stream-text">
              <span>Tests pass after the targeted fix.</span>
              <span>Workspace context stays attached.</span>
              <span>The diff is ready for review.</span>
            </p>
            <div className="mini-tool-row">
              <span>workspace</span>
              <span>test output</span>
              <span>diff ready</span>
            </div>
            <p className="mini-model-switch-note"><span>model switched</span><code>thread retained</code></p>
          </div>
          <div className="mini-diff-card">
            <div className="mini-diff-head">
              <span>src/review.ts</span>
              <code>+3 −1</code>
            </div>
            <pre>
              <span className="mini-diff-context">@@ -42,4 +42,6 @@ reviewDiff</span>
              <span className="mini-diff-remove">- return publish(change)</span>
              <span className="mini-diff-add">+ const review = await inspect(change)</span>
              <span className="mini-diff-add">+ if (!review.approved) return hold(change)</span>
              <span className="mini-diff-add">+ return publish(review.patch)</span>
            </pre>
            <div className="mini-diff-review">
              <span>inline review ready</span>
              <em>Reject</em>
              <em className="mini-review-approve">Approve</em>
            </div>
          </div>
          <div className="mini-composer-card">
            <div className="mini-control-bar">
              <div className="mini-chips">
                <span className="mini-chip mini-model-chip">
                  <i />
                  <span className="mini-model-label mini-model-a">ollama/llama3.2</span>
                  <span className="mini-model-label mini-model-b">openai/gpt-5</span>
                  <MiniChevronDownIcon size={10} />
                </span>
                <span className="mini-chip mini-plan-chip">
                  <MiniLightbulbIcon size={12} />
                  <span>Plan</span>
                  <em>Read-only</em>
                </span>
                <span className="mini-chip mini-goal-chip">
                  <MiniPinIcon size={12} />
                  <span>Goal</span>
                </span>
              </div>
            </div>
            <div className="mini-composer-input">
              <span>Switch models, review the diff, then approve.</span>
            </div>
            <div className="mini-composer-bar">
              <div className="mini-composer-tools">
                <span className="mini-project-chip"><MiniFolderIcon size={13} /> No project <MiniChevronDownIcon size={10} /></span>
                <span><MiniPaperclipIcon size={15} /></span>
                <span><MiniSlashIcon size={15} /></span>
                <span><MiniUserRoundIcon size={13} /></span>
              </div>
              <div className="mini-composer-send">
                <b><MiniArrowUpIcon size={17} /></b>
              </div>
            </div>
          </div>
        </section>
      </div>
    </div>
  );
}

function ReleaseObject() {
  return (
    <div className="download-media release-object" aria-label="milim source and release concept" role="img">
      <div className="release-grid" aria-hidden="true">
        <span>git clone oshtz/milim</span>
        <span>pnpm build</span>
        <span>cargo tauri build</span>
        <span>license: MIT</span>
      </div>
      <div className="release-card">
        <img src="/assets/milim-wordmark.svg" alt="" />
        <div>
          <strong>source first</strong>
          <span>stable download aliases on every release</span>
        </div>
      </div>
    </div>
  );
}

function HeroBackgroundEffect({ dither = true }: { dither?: boolean }) {
  return (
    <>
      <ShaderField dither={dither} />
      <HeroAsciiField />
    </>
  );
}

function Nav() {
  return (
    <header className="nav">
      <a className="brand" href="/" aria-label="milim home">
        <img src="/assets/milim-wordmark.svg" alt="" />
      </a>
      <div className="nav-actions">
        <nav className="primary-nav" aria-label="Primary">
          {navLinks.map((link) => (
            <a className={link.className} href={link.href} key={link.href}>
              {link.label}
            </a>
          ))}
        </nav>
        <ThemeControl />
        <SiteMobileNav links={navLinks} />
      </div>
    </header>
  );
}

function Footer() {
  return (
    <footer className="footer">
      <div className="footer-main">
        <a className="footer-mark" href="/" aria-label="milim home">
          <img src="/assets/milim-wordmark.svg" alt="" />
        </a>
        <p>&copy; {new Date().getFullYear()} milim contributors. MIT licensed.</p>
      </div>
      <nav className="footer-nav" aria-label="Footer">
        <a href={DOCS_URL}>Docs</a>
        <a href={DOCS_QUICKSTART_URL}>Quickstart</a>
        <a href={`${DOCS_URL}models`}>Providers</a>
        <a href={`${DOCS_URL}privacy`}>Privacy</a>
        <a href={RELEASES_URL}>Latest release</a>
        <a href={`${GITHUB_URL}/releases`}>Changelog</a>
        <a href={GITHUB_URL}>GitHub</a>
        <a href={`${GITHUB_URL}/blob/main/LICENSE`}>License</a>
      </nav>
    </footer>
  );
}

function ChapterVisual({ kind }: { kind: ChapterKind }) {
  const { label, foot } = chapterVisuals[kind];
  return (
    <div className={`chapter-visual chapter-visual-${kind}`} aria-hidden="true">
      <div className="chapter-visual-bar">
        <span>{label}</span>
        <i />
      </div>
      <div className="chapter-visual-body">
        {kind === "models" ? (
          <div className="route-rail">
            <span className="route-cursor" />
            {routeSources.map((source) => (
              <span className="route-row" key={source.name}>
                <b>{source.name}</b>
                <em>{source.tag}</em>
              </span>
            ))}
          </div>
        ) : null}
        {kind === "privacy" ? (
          <div className="gate-lanes">
            <span className="gate-line" />
            <span className="gate-lane gate-lane-local">
              <b>local</b>
              <s>127.0.0.1:11434</s>
              <em>untouched</em>
            </span>
            <span className="gate-lane gate-lane-remote">
              <b>remote</b>
              <s>email: <u>dev@example.test<i /></u></s>
              <em>redacted</em>
            </span>
          </div>
        ) : null}
        {kind === "tools" ? (
          <ol className="run-timeline">
            {toolSteps.map((step) => (
              <li key={step.name}>
                <b>{step.name}</b>
                <s>{step.meta}</s>
                <em>{step.detail}</em>
              </li>
            ))}
          </ol>
        ) : null}
        {kind === "memory" ? (
          <div className="recall">
            <p className="recall-query">where is auth handled?</p>
            <ul className="recall-hits">
              {recallHits.map((hit) => (
                <li key={hit.path}>
                  <b>{hit.path}</b>
                  <i />
                  <s>{hit.score}</s>
                </li>
              ))}
            </ul>
          </div>
        ) : null}
      </div>
      <p className="chapter-visual-foot">{foot}</p>
    </div>
  );
}

function DownloadIcon() {
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24">
      <path d="M12 3v11m0 0 4-4m-4 4-4-4M5 18h14" />
    </svg>
  );
}

type MiniIconProps = SVGProps<SVGSVGElement> & { size?: number };

function MiniSvg({ size = 16, children, ...rest }: MiniIconProps & { children: ReactNode }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.8}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      {...rest}
    >
      {children}
    </svg>
  );
}

function MiniLightbulbIcon(p: MiniIconProps) {
  return <MiniSvg {...p}><path d="M9 18h6M10 22h4M12 2a7 7 0 0 0-4 12.7c.6.5 1 1.3 1 2.1h6c0-.8.4-1.6 1-2.1A7 7 0 0 0 12 2Z" /></MiniSvg>;
}

function MiniPaperclipIcon(p: MiniIconProps) {
  return <MiniSvg {...p}><path d="M21 11.5 12.5 20a4.5 4.5 0 0 1-6.4-6.4l8.5-8.5a3 3 0 0 1 4.3 4.3l-8.6 8.5a1.5 1.5 0 0 1-2.1-2.1l7.8-7.8" /></MiniSvg>;
}

function MiniSlashIcon(p: MiniIconProps) {
  return <MiniSvg {...p}><path d="M9 20 15 4" /></MiniSvg>;
}

function MiniUserRoundIcon(p: MiniIconProps) {
  return <MiniSvg {...p}><circle cx="12" cy="8" r="4" /><path d="M4 21a8 8 0 0 1 16 0" /></MiniSvg>;
}

function MiniArrowUpIcon(p: MiniIconProps) {
  return <MiniSvg {...p}><path d="M12 19V5M6 11l6-6 6 6" /></MiniSvg>;
}

function MiniGearIcon(p: MiniIconProps) {
  return <MiniSvg {...p}><circle cx="12" cy="12" r="3" /><path d="M19.4 13.5a1.7 1.7 0 0 0 .3 1.9l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.9-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 0 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.9.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.9 1.7 1.7 0 0 0-1.5-1H3a2 2 0 0 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.9l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.9.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 0 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.9-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.9V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 0 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1Z" /></MiniSvg>;
}

function MiniPinIcon(p: MiniIconProps) {
  return <MiniSvg {...p}><path d="M9 4h6l-1 6 3 3v2H7v-2l3-3-1-6ZM12 15v5" /></MiniSvg>;
}

function MiniSidebarIcon(p: MiniIconProps) {
  return <MiniSvg {...p}><rect x="3" y="4" width="18" height="16" rx="2" /><path d="M9 4v16" /></MiniSvg>;
}

function MiniChevronDownIcon(p: MiniIconProps) {
  return <MiniSvg {...p}><path d="m6 9 6 6 6-6" /></MiniSvg>;
}

function MiniFolderIcon(p: MiniIconProps) {
  return <MiniSvg {...p}><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z" /></MiniSvg>;
}

function MiniPlusIcon(p: MiniIconProps) {
  return <MiniSvg {...p}><path d="M12 5v14M5 12h14" /></MiniSvg>;
}

function MiniSearchIcon(p: MiniIconProps) {
  return <MiniSvg {...p}><circle cx="11" cy="11" r="7" /><path d="m21 21-4.3-4.3" /></MiniSvg>;
}

function MiniCalendarIcon(p: MiniIconProps) {
  return <MiniSvg {...p}><rect x="4" y="5" width="16" height="15" rx="2" /><path d="M8 3v4M16 3v4M4 10h16M8 14h2M14 14h2M8 17h2" /></MiniSvg>;
}

function MiniWindowMinIcon() {
  return <svg width="11" height="11" viewBox="0 0 16 16" aria-hidden="true"><path d="M3 8h10" stroke="currentColor" strokeWidth="1.3" /></svg>;
}

function MiniWindowMaxIcon() {
  return <svg width="11" height="11" viewBox="0 0 16 16" aria-hidden="true"><rect x="3.5" y="3.5" width="9" height="9" rx="1.5" fill="none" stroke="currentColor" strokeWidth="1.3" /></svg>;
}

function MiniWindowCloseIcon() {
  return <svg width="11" height="11" viewBox="0 0 16 16" aria-hidden="true"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" /></svg>;
}

function ArrowIcon() {
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24">
      <path d="M7 17 17 7m0 0H9m8 0v8" />
    </svg>
  );
}
