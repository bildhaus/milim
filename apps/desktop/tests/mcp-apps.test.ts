import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";
import type { ChatStreamPart, McpAppDescriptor } from "../src/api.js";
import {
  NativeChartView,
  barMarkPath,
  chartCategoryTickIndexes,
  chartNumericTickIndexes,
  formatChartValue,
  niceChartTicks,
  parseNativeChartSpec,
} from "../src/components/NativeChartView.js";
import { mcpAppFallbackText, parseMcpAppArguments } from "../src/lib/mcpApps.js";
import { groupCompletedStreamActivity } from "../src/lib/streamParts.js";
import { accountRuntimeToolPart, toolCompletedPart } from "../src/lib/turnEvents.js";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

const descriptor: McpAppDescriptor = {
  server_id: "fixture",
  resource_uri: "ui://fixture/chart",
  tool: { name: "show_chart", inputSchema: { type: "object" } },
};

assert(parseMcpAppArguments('{"value":1}').value === 1, "tool arguments should parse");
assert(Object.keys(parseMcpAppArguments("not json")).length === 0, "invalid arguments should be empty");
assert(
  mcpAppFallbackText({ content: [{ type: "text", text: "Fallback chart data" }] }) === "Fallback chart data",
  "text fallback should remain available when the app cannot load",
);

const appPart = toolCompletedPart({
  type: "tool_result",
  name: "show_chart",
  call_id: "call-1",
  arguments: "{}",
  result: { content: [{ type: "text", text: "fallback" }] },
  mcp_app: descriptor,
  mcp_app_result: { structuredContent: { values: [1, 2] }, content: [] },
});
assert(appPart.mcpApp === descriptor, "tool event should retain the app descriptor");
assert(appPart.mcpAppResult != null, "tool event should retain the full app result");
const parts: ChatStreamPart[] = [
  { kind: "event", eventType: "tool", label: "Used first", status: "done" },
  appPart,
  { kind: "event", eventType: "tool", label: "Used last", status: "done" },
];
const grouped = groupCompletedStreamActivity(parts, false);
assert(grouped.length === 2, "ordinary tool activity should share one drawer around an MCP App");
assert(grouped[0].kind === "workGroup", "ordinary tools should use the completed work drawer");
assert(grouped[1].kind === "event" && grouped[1].mcpApp === descriptor, "MCP App should remain visible outside the work drawer");

const server = await createServer({
  root: process.cwd(),
  appType: "custom",
  logLevel: "silent",
  server: { middlewareMode: true },
});
try {
  const { McpAppView } = await server.ssrLoadModule("/src/components/McpAppView.tsx");
  const legacyAppMarkup = renderToStaticMarkup(createElement(McpAppView, {
    descriptor: { server_id: "legacy", resource_uri: "ui://legacy" } as McpAppDescriptor,
    approval: "review",
  }));
  assert(legacyAppMarkup.includes("Interactive view MCP App"), "legacy MCP App entries without tool metadata should not crash the transcript");
} finally {
  await server.close();
}

const chart = {
  title: "Weekly usage",
  type: "line",
  series: [{ name: "Requests", points: [{ x: "Mon", y: 12 }, { x: "Tue", y: 18 }] }],
};
assert(parseNativeChartSpec(chart)?.series[0].points.length === 2, "valid native chart data should parse");
assert(parseNativeChartSpec({ ...chart, type: "scatter" }) === null, "scatter charts should require numeric x values");
assert(parseNativeChartSpec({ ...chart, type: "bar", orientation: "horizontal" })?.orientation === "horizontal", "bar charts should accept horizontal orientation");
assert(parseNativeChartSpec({ ...chart, orientation: "horizontal" }) === null, "non-bar charts should reject orientation");
assert(parseNativeChartSpec({ ...chart, type: "bar", orientation: "diagonal" }) === null, "bar charts should reject unknown orientations");
const formattedChart = parseNativeChartSpec({
  ...chart,
  y_format: { style: "percent", precision: 1, sign_display: "always" },
});
assert(formattedChart?.y_format?.style === "percent", "valid number formats should parse");
assert(parseNativeChartSpec({ ...chart, y_format: { style: "currency" } }) === null, "currency formats should require a currency code");
assert(parseNativeChartSpec({ ...chart, y_format: { style: "number", precision: 5 } }) === null, "format precision should stay bounded");
assert(parseNativeChartSpec({ ...chart, extra: true }) === null, "native chart parsing should reject unknown fields like the Rust validator");
assert(formatChartValue(2.5, { style: "percent", precision: 1 }) === "2.5%", "percent values should use percentage-point semantics");
assert(formatChartValue(2.5, { style: "percent", precision: 1, sign_display: "always" }).startsWith("+"), "signed formats should show positive signs");
assert(/USD|\$/.test(formatChartValue(1250, { style: "currency", currency: "USD", notation: "compact", precision: 1 })), "compact currency should retain its currency marker");
assert(JSON.stringify(niceChartTicks(3, 97)) === JSON.stringify([0, 20, 40, 60, 80, 100]), "nice ticks should use readable 1/2/5 steps");
const narrowTickIndexes = chartCategoryTickIndexes(["Very long first category", "Second category", "Third category", "Very long last category"], 180);
assert(narrowTickIndexes[0] === 0 && narrowTickIndexes.at(-1) === 3 && narrowTickIndexes.length < 4, "narrow charts should retain edge labels while reducing tick density");
assert(JSON.stringify(chartNumericTickIndexes([-15, -10, -5, 0, 5, 10, 15], ["-15.0%", "-10.0%", "-5.0%", "+0.0%", "+5.0%", "+10.0%", "+15.0%"], 194)) === JSON.stringify([0, 3, 6]), "narrow numeric axes should retain endpoints and zero without crowding");

const chartPart = toolCompletedPart({
  type: "tool_result",
  name: "render_chart",
  call_id: "chart-1",
  arguments: JSON.stringify(chart),
  result: chart,
  mcp_app: { kind: "native_chart" },
  mcp_app_result: chart,
});
const chartGroup = groupCompletedStreamActivity([
  { kind: "event", eventType: "tool", label: "Used first", status: "done" },
  chartPart,
], false);
assert(chartGroup.length === 2 && chartGroup[1].kind === "event", "native charts should remain at their exact transcript position");

const accountChart = accountRuntimeToolPart({
  type: "tool_started",
  id: "chart-2",
  name: "mcp__milim__render_chart",
  status: "running",
  detail: JSON.stringify(chart),
});
assert(accountChart.mcpApp?.kind === "native_chart", "account runtime chart calls should use the native renderer");
assert(accountChart.toolArguments === JSON.stringify(chart), "account runtime chart arguments should reach the renderer");

const barMarkup = renderToStaticMarkup(createElement(NativeChartView, {
  result: {
    title: "Weekly change",
    type: "bar",
    y_format: { style: "percent", precision: 1, sign_display: "always" },
    series: [{ name: "Change", points: [{ x: "First", y: 20 }, { x: "Last", y: -10 }] }],
  },
}));
const bars = [...barMarkup.matchAll(/<path\b[^>]*data-chart-mark="true"/g)];
assert(bars.length === 2, "bar charts should render one path per point");
assert(barMarkPath(100, 100, 20, 40, 3, "top").endsWith("V 140 H 100 Z"), "positive vertical bars should stay square at their baseline");
assert(barMarkPath(100, 100, 20, 40, 3, "bottom").startsWith("M 100 100 H 120"), "negative vertical bars should stay square at their baseline");
assert((barMarkup.match(/native-chart-value-label/g) ?? []).length === 2, "sparse bars should show direct values");
assert(barMarkup.includes("%"), "visible and accessible chart values should use the requested format");
assert(barMarkup.includes("<linearGradient") && barMarkup.includes("color-mix(in srgb") && barMarkup.includes("fill=\"url(#"), "bar charts should use visible accent-derived gradients");
assert(barMarkup.includes("native-chart-bar-mark reveal-vertical") && barMarkup.includes("animation-delay:22ms"), "vertical bars should reveal from zero with a short stagger");
assert(!barMarkup.includes("<title"), "custom chart tooltips should not compete with browser-native SVG tooltips");

const horizontalBarMarkup = renderToStaticMarkup(createElement(NativeChartView, {
  result: {
    title: "Ranked change",
    type: "bar",
    orientation: "horizontal",
    x_label: "Category",
    y_label: "Change",
    series: [{ name: "Change", points: [{ x: "Long first category", y: 20 }, { x: "Long last category", y: -10 }] }],
  },
}));
const horizontalBars = [...horizontalBarMarkup.matchAll(/<path\b[^>]*data-chart-mark="true"/g)];
assert(horizontalBarMarkup.includes('data-orientation="horizontal"'), "horizontal bars should expose their rendered orientation");
assert(horizontalBars.length === 2, "horizontal bars should render one bounded mark per point");
assert(barMarkPath(100, 100, 40, 20, 3, "right").endsWith("H 100 Z"), "positive horizontal bars should stay square at their baseline");
assert(horizontalBarMarkup.includes("native-chart-bar-mark reveal-horizontal"), "horizontal bars should reveal from zero");
assert((horizontalBarMarkup.match(/native-chart-value-label/g) ?? []).length === 2, "sparse horizontal bars should retain direct values");
assert(horizontalBarMarkup.includes("Use up and down to move within a series"), "horizontal bars should explain physical-direction keyboard navigation");

const interactiveMarkup = renderToStaticMarkup(createElement(NativeChartView, {
  result: {
    title: "Interactive line",
    type: "line",
    series: [
      { name: "Current", points: [{ x: "Mon", y: 12 }, { x: "Tue", y: 18 }] },
      { name: "Previous", points: [{ x: "Mon", y: 9 }, { x: "Tue", y: 14 }] },
    ],
  },
}));
assert((interactiveMarkup.match(/data-chart-mark="true"/g) ?? []).length === 4, "every chart point should be interactive");
assert(interactiveMarkup.includes('role="graphics-document"'), "interactive SVG charts should expose their child marks");
assert((interactiveMarkup.match(/tabindex="0"/g) ?? []).length === 1, "a chart should expose one keyboard entry point");
assert(interactiveMarkup.includes('tabindex="-1"'), "arrow-key targets should use roving tab stops");
assert((interactiveMarkup.match(/aria-pressed="true"/g) ?? []).length === 2, "legend buttons should expose visible series state");
assert(interactiveMarkup.includes("Use left and right to move within a series"), "charts should explain two-dimensional keyboard navigation");
assert(interactiveMarkup.includes("native-chart-hit"), "line and scatter marks should expose larger pointer targets");
assert(interactiveMarkup.includes("stroke-dasharray=\"7 4\""), "line series should retain a non-color distinction");
