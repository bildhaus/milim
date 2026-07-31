import type { SkillInfo, ToolInfo } from "../src/api.js";
import { composerDisplayForText, composerLinkClickAction, composerTokenParts, composerTokensForText, pasteComposerUrl } from "../src/lib/composerTokens.js";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function equal<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
}

function skill(name: string, enabled = true): SkillInfo {
  return {
    id: `skill-${name.toLowerCase().replace(/\s+/g, "-")}`,
    name,
    description: "",
    instructions: "",
    enabled,
    source_kind: "builtin",
  };
}

function tool(name: string): ToolInfo {
  return { name, description: "", effect: "unknown" };
}

const skills = [
  skill("Code"),
  skill("Code Review"),
  skill("Design-Polish"),
  skill("Disabled Skill", false),
];
const tools = [tool("github__search"), tool("filesystem__read_file")];

const tagged = composerTokensForText("Please run @Code Review and /Design-Polish.", { skills, tools });
equal(tagged.length, 2, "skill tags should be detected");
equal(tagged[0]?.kind, "skill", "mention token kind");
equal(tagged[0]?.label, "Code Review", "longest skill name wins");
equal(tagged[0]?.start, "Please run ".length, "mention token start");
equal(tagged[1]?.label, "Design-Polish", "slash skill with hyphen");

const ignored = composerTokensForText("@Disabled Skill @Unknown Skill", { skills, tools });
equal(ignored.length, 0, "disabled and unknown skill tags should be ignored");

const mcp = composerTokensForText("Use /github__search then /missing__tool", { skills, tools });
equal(mcp.length, 1, "known MCP tool should be detected");
equal(mcp[0]?.kind, "mcp", "MCP token kind");
equal(mcp[0]?.value, "github__search", "MCP token value");

const files = composerTokensForText("Open @src/App.tsx and @\"docs/My File.md\"", { skills, tools });
equal(files.length, 2, "workspace file tags should be detected");
equal(files[0]?.kind, "file", "unquoted file token kind");
equal(files[0]?.value, "src/App.tsx", "unquoted file token value");
equal(files[1]?.value, "docs/My File.md", "quoted file token value");

const links = composerTokensForText("See https://milim.ai/docs.", { skills, tools });
equal(links.length, 1, "bare URL should be detected");
equal(links[0]?.kind, "link", "link token kind");
equal(links[0]?.value, "https://milim.ai/docs", "trailing punctuation should stay outside URL token");

const githubLink = composerTokensForText("See https://github.com/oshtz/milim/issues/12?notification_referrer_id=1.", { skills, tools });
equal(githubLink[0]?.label, "oshtz/milim/issues/12", "GitHub links should hide the origin and query in their display label");
equal(githubLink[0]?.value, "https://github.com/oshtz/milim/issues/12?notification_referrer_id=1", "GitHub link tokens should preserve the full URL");
equal(composerTokensForText("Keep https://[ as text", { skills, tools })[0]?.label, "https://[", "malformed URLs should keep their original label");
equal(composerLinkClickAction({ ctrlKey: true, metaKey: false, shiftKey: false }), "external", "Ctrl-click should open links externally");
equal(composerLinkClickAction({ ctrlKey: true, metaKey: false, shiftKey: true }), "sidepanel", "Ctrl-Shift-click should open links in the sidepanel");
equal(composerLinkClickAction({ ctrlKey: false, metaKey: true, shiftKey: false }), "external", "Command-click should match Ctrl-click");
equal(composerLinkClickAction({ ctrlKey: false, metaKey: false, shiftKey: true }), null, "Shift-click alone should remain a selection gesture");

const githubPrompt = "Review https://github.com/oshtz/milim then reply";
const githubDisplay = composerDisplayForText(githubPrompt, composerTokensForText(githubPrompt));
const githubDisplayLabel = [..."oshtz/milim"].join("\u200B");
equal(githubDisplay.text, `Review \uFFFC ${githubDisplayLabel} then reply`, "GitHub tokens should occupy only their compact display footprint");
equal(githubDisplay.rawOffset(`Review \uFFFC ${githubDisplayLabel}`.length), "Review https://github.com/oshtz/milim".length, "display offsets should map past the full raw URL");
equal(githubDisplay.displayOffset("Review https://github.com/oshtz/milim".length), `Review \uFFFC ${githubDisplayLabel}`.length, "raw offsets should map past the compact token");
assert(!githubDisplay.text.includes("oshtz"), "the display footprint should break link words for spellcheck");
equal(githubDisplay.applyEdit(`${githubDisplay.text}!`).value, `${githubPrompt}!`, "typing after a compact token should edit the raw prompt at the same visual position");
equal(githubDisplay.applyEdit("Review  then reply").value, "Review  then reply", "editing a compact token should remove the full raw URL atomically");
const pastedUrl = pasteComposerUrl("Review ", 7, 7, "https://github.com/oshtz/milim");
equal(pastedUrl?.value, "Review https://github.com/oshtz/milim ", "pasted URLs should receive a trailing space");
equal(pastedUrl?.cursor, "Review https://github.com/oshtz/milim ".length, "the cursor should move after a pasted URL's trailing space");
equal(pasteComposerUrl("", 0, 0, "not a URL"), null, "ordinary pasted text should keep native paste behavior");

const overlap = composerTokensForText("Open https://example.test/github__search and @Code Review", { skills, tools });
equal(overlap.length, 2, "link and later skill should both be detected");
equal(overlap[0]?.kind, "link", "URL should not be split by slash-like text");

const prompt = "Inspect @Code Review with /github__search at https://milim.ai";
const tokens = composerTokensForText(prompt, { skills, tools });
const parts = composerTokenParts(prompt, tokens);
assert(parts.some((part) => part.kind === "token" && part.token.kind === "mcp"), "highlight parts should include token spans");
equal(parts.map((part) => part.text).join(""), prompt, "highlight parts should preserve textarea value exactly");
