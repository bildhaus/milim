// Renders real diagrams through MermaidDiagram in headless Chrome. Mermaid
// lays diagrams out with SVG text measurement, which Node and the SSR tests in
// markdown-rendering.test.tsx cannot exercise, so this mounts the component in
// a page served by the app's own Vite config and waits for the lazy renderer.
//
// Uses an installed Google Chrome (preinstalled on GitHub's ubuntu runners).
// Without one, the test is skipped locally and fails on CI.
import { chromium, type Browser } from "playwright-core";
import { createServer, type Plugin } from "vite";
import type { AddressInfo } from "node:net";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

type Rendered = {
  status: "ready" | "error";
  svg: string;
  error: string;
  width: number;
  height: number;
  text: string;
};

const HARNESS_ID = "virtual:milim-mermaid-harness";
const RESOLVED_HARNESS_ID = `\0${HARNESS_ID}`;
const HARNESS_PATH = "/__mermaid-harness.html";

// Mounts one <MermaidDiagram> per call and resolves once it shows an SVG or an
// error, the same two end states the transcript card can reach.
const harnessModule = `
import { createElement } from "react";
import { createRoot } from "react-dom/client";
import { MermaidDiagram } from "/src/components/MermaidDiagram.tsx";
import { useTheme } from "/src/theme/store.ts";

window.milimThemeColors = useTheme.getState().theme.colors;
window.milimRenderMermaid = (source) => new Promise((resolve, reject) => {
  const container = document.createElement("div");
  document.body.append(container);
  createRoot(container).render(createElement(MermaidDiagram, { source }));
  const started = performance.now();
  const poll = () => {
    const svg = container.querySelector(".mermaid-svg svg");
    const error = container.querySelector(".mermaid-error");
    if (svg || error) {
      const box = svg?.getBoundingClientRect();
      resolve({
        status: svg ? "ready" : "error",
        svg: svg?.outerHTML ?? "",
        error: error?.textContent ?? "",
        width: box?.width ?? 0,
        height: box?.height ?? 0,
        text: svg?.textContent ?? "",
      });
    } else if (performance.now() - started > 30000) {
      reject(new Error("MermaidDiagram did not finish rendering within 30 s"));
    } else {
      setTimeout(poll, 25);
    }
  };
  poll();
});
window.milimHarnessReady = true;
`;

const harnessPlugin: Plugin = {
  name: "milim-mermaid-harness",
  resolveId: (id) => (id === HARNESS_ID ? RESOLVED_HARNESS_ID : undefined),
  load: (id) => (id === RESOLVED_HARNESS_ID ? harnessModule : undefined),
  configureServer(server) {
    server.middlewares.use((req, res, next) => {
      if (req.url !== HARNESS_PATH) return next();
      res.setHeader("Content-Type", "text/html");
      res.end(`<!doctype html><html><head><meta charset="utf-8"><link rel="icon" href="data:,"></head><body>
<script type="module" src="/@id/__x00__${HARNESS_ID}"></script></body></html>`);
    });
  },
};

let browser: Browser | undefined;
try {
  browser = await chromium.launch({ channel: "chrome", headless: true });
} catch (error) {
  if (process.env.CI) throw error;
  console.log(`Skipping Mermaid browser rendering: Google Chrome is unavailable (${String(error).split("\n")[0]})`);
}

if (browser) {
  const server = await createServer({
    root: process.cwd(),
    logLevel: "silent",
    clearScreen: false,
    plugins: [harnessPlugin],
    server: { host: "127.0.0.1", port: 0, strictPort: false, hmr: false, watch: null },
    // A private cache keeps the other Vite-backed tests, which run in
    // parallel, from re-optimizing dependencies under this page and loading a
    // second Mermaid instance that never saw initialize(). Pre-bundling the
    // component's graph up front keeps a mid-test discovery from doing the same.
    cacheDir: "node_modules/.vite-mermaid-rendering",
    optimizeDeps: { entries: ["src/components/MermaidDiagram.tsx"], include: ["react-dom/client"] },
  });
  try {
    await server.listen();
    const { port } = server.httpServer!.address() as AddressInfo;
    const page = await browser.newPage();
    const pageErrors: string[] = [];
    page.on("pageerror", (error) => pageErrors.push(error.message));
    page.on("console", (message) => {
      if (message.type() === "error") pageErrors.push(message.text());
    });
    await page.goto(`http://127.0.0.1:${port}${HARNESS_PATH}`);
    await page.waitForFunction(() => (window as unknown as { milimHarnessReady?: boolean }).milimHarnessReady, null, { timeout: 120_000 });
    const colors = await page.evaluate(() => (window as unknown as { milimThemeColors: Record<string, string> }).milimThemeColors);
    const render = (source: string) => page.evaluate(
      (value) => (window as unknown as { milimRenderMermaid: (source: string) => Promise<Rendered> }).milimRenderMermaid(value),
      source,
    );

    const expectDiagram = async (kind: string, source: string, labels: string[]) => {
      const result = await render(source);
      assert(result.status === "ready", `${kind} should render, got error: ${result.error}`);
      assert(result.svg.startsWith("<svg"), `${kind} should produce an SVG`);
      assert(result.width > 0 && result.height > 0, `${kind} should lay out to a non-empty SVG (${result.width}x${result.height})`);
      for (const label of labels) assert(result.text.includes(label), `${kind} should draw the "${label}" label`);
      assert(!/<script|\son[a-z]+=|javascript:/i.test(result.svg), `${kind} SVG should not carry script content`);
      return result;
    };

    const flowchart = await expectDiagram("flowchart", [
      "flowchart LR",
      "  Start[Request] --> Check{Cached?}",
      "  Check -->|yes| Serve[Serve cache]",
      "  Check -->|no| Fetch[Fetch origin]",
      "  subgraph Origin",
      "    Fetch --> Store[(Store)]",
      "  end",
    ].join("\n"), ["Request", "Cached?", "Serve cache", "Fetch origin", "Origin"]);
    // The app passes theme "base" with its own palette and pins the classic
    // look; Mermaid 12 would otherwise apply redux-color/neo to flowcharts.
    assert(
      flowchart.svg.toLowerCase().includes(colors.bgTertiary.toLowerCase()),
      `flowchart should use the app theme's primaryColor ${colors.bgTertiary}`,
    );
    assert(!flowchart.svg.includes("<foreignObject"), "flowchart labels should stay SVG text (htmlLabels: false)");
    assert(flowchart.svg.includes('data-look="classic"'), "flowchart nodes should use the classic look");
    assert(!/<[a-z]+\b[^>]*\sdata-look="neo"/.test(flowchart.svg), "flowchart nodes should not switch to Mermaid 12's neo look");

    await expectDiagram("sequence diagram", [
      "sequenceDiagram",
      "  participant User",
      "  participant Milim",
      "  User->>Milim: Send prompt",
      "  Milim-->>User: Stream answer",
      "  Note over User,Milim: Rendered in Chrome",
    ].join("\n"), ["User", "Milim", "Send prompt", "Stream answer", "Rendered in Chrome"]);

    await expectDiagram("class diagram", [
      "classDiagram",
      "  class Thread {",
      "    +String id",
      "    +send(message)",
      "  }",
      "  Thread \"1\" --> \"*\" Message : contains",
    ].join("\n"), ["Thread", "Message", "contains"]);

    await expectDiagram("state diagram", [
      "stateDiagram-v2",
      "  [*] --> Idle",
      "  Idle --> Running: start",
      "  Running --> Idle: stop",
      "  Running --> [*]",
    ].join("\n"), ["Idle", "Running", "start", "stop"]);

    // securityLevel "strict" must keep stripping links and event handlers.
    const hostile = await expectDiagram("hostile flowchart", [
      "flowchart LR",
      "  A[\"<img src=x onerror=alert(1)>Click\"] --> B",
      "  click A \"javascript:alert(1)\"",
    ].join("\n"), ["B"]);
    assert(!/<img|<foreignObject/i.test(hostile.svg), "labels should stay SVG text and never emit HTML");

    const invalid = await render("flowchart ???");
    assert(invalid.status === "error", "invalid Mermaid source should show the error state");
    assert(invalid.error.trim().length > 0, "invalid Mermaid source should explain the failure");

    assert(pageErrors.length === 0, `the harness page should not log errors:\n${pageErrors.join("\n")}`);
    console.log("Rendered flowchart, sequence, class and state diagrams in Chrome");
  } finally {
    await browser.close();
    await server.close();
  }
}
