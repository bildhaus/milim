import { composerAutocompleteTriggerAt, composerCommandRunsOnSelection, composerSuggestionMatchScore, mcpToolTagCompletion, replaceComposerAutocompleteTrigger, skillTagCompletion } from "../src/lib/composerAutocomplete.js";
import { isLoopbackProviderEndpoint } from "../src/lib/providerEndpoint.js";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function equal<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
}

const midSlash = "Please use /mo";
const slash = composerAutocompleteTriggerAt(midSlash, midSlash.length);
assert(slash, "slash trigger should be detected away from the first character");
equal(slash.prefix, "/", "slash trigger prefix");
equal(slash.query, "mo", "slash trigger query");
equal(replaceComposerAutocompleteTrigger(midSlash, slash, "/model "), "Please use /model ", "slash replacement");

const midMention = "Check @src/App";
const mention = composerAutocompleteTriggerAt(midMention, midMention.length);
assert(mention, "mention trigger should be detected away from the first character");
equal(mention.prefix, "@", "mention trigger prefix");
equal(mention.query, "src/app", "mention query is lowercase");
equal(replaceComposerAutocompleteTrigger(midMention, mention, "@src/App.tsx "), "Check @src/App.tsx ", "mention replacement");

const slashSkill = "Use /des";
const skill = composerAutocompleteTriggerAt(slashSkill, slashSkill.length);
assert(skill, "slash trigger should support skill tags");
equal(replaceComposerAutocompleteTrigger(slashSkill, skill, skillTagCompletion("/", "Design-Polish")), "Use /Design-Polish ", "slash skill replacement");
equal(skillTagCompletion("@", "Code Review"), "@Code Review ", "mention skill completion");

const slashMcp = "Use /github";
const mcp = composerAutocompleteTriggerAt(slashMcp, slashMcp.length);
assert(mcp, "slash trigger should support MCP tool tags");
equal(replaceComposerAutocompleteTrigger(slashMcp, mcp, mcpToolTagCompletion("github__search")), "Use /github__search ", "MCP slash replacement");

equal(composerAutocompleteTriggerAt("email ada@example.com", "email ada@example.com".length), null, "email should not trigger mention");
equal(composerAutocompleteTriggerAt("run/path", "run/path".length), null, "slash inside a token should not trigger commands");
equal(composerCommandRunsOnSelection("plan"), true, "plan should activate as soon as it is selected");
equal(composerCommandRunsOnSelection("goal"), true, "goal should activate as soon as it is selected");
equal(composerCommandRunsOnSelection("model"), false, "argument-taking settings should remain in the composer");
equal(composerSuggestionMatchScore("model picker", "mo"), 0, "prefix matches should rank first");
equal(composerSuggestionMatchScore("choose model", "mo"), 1, "token-prefix matches should rank second");
assert((composerSuggestionMatchScore("remote model", "emote") ?? 0) >= 2, "substring matches should rank after token matches");
equal(composerSuggestionMatchScore("model picker", "xyz"), null, "non-matches should be excluded");
equal(isLoopbackProviderEndpoint("http://127.0.0.1:11434/v1"), true, "IPv4 loopback providers should be local");
equal(isLoopbackProviderEndpoint("http://localhost:1234/v1"), true, "localhost providers should be local");
equal(isLoopbackProviderEndpoint("https://api.openai.com/v1"), false, "hosted providers should not be local");
