import { equal } from "node:assert/strict";
import { connectedSourceModel } from "../src/lib/onboardingModel.js";
import { RUNTIME_INSTALL_COMMANDS, runtimeMissingMessage } from "../src/lib/runtimeInstall.js";

const old = { id: "old", owned_by: "OpenAI", provider_id: "old-provider" };
const connected = { id: "new", owned_by: "OpenAI", provider_id: "new-provider" };
equal(connectedSourceModel([old], { providerId: "new-provider" }), undefined, "a cached unrelated source must not be selected while refresh is pending");
equal(connectedSourceModel([old, connected], { providerId: "new-provider" }), connected, "same-brand providers must still be selected by exact provider id");
equal(connectedSourceModel([old], { owner: "Codex" }), undefined, "an unavailable account runtime must not fall back to another source");
const codex = { id: "codex:model", owned_by: "Codex" };
equal(connectedSourceModel([old, codex], { owner: "codex" }), codex);

equal(RUNTIME_INSTALL_COMMANDS.codex, "npm install -g @openai/codex");
equal(RUNTIME_INSTALL_COMMANDS.claude, "npm install -g @anthropic-ai/claude-code");
equal(RUNTIME_INSTALL_COMMANDS.opencode, "npm install -g opencode-ai");
equal(RUNTIME_INSTALL_COMMANDS.pi, "npm install -g @earendil-works/pi-coding-agent");
for (const kind of ["codex", "claude", "opencode", "pi"] as const) {
  const message = runtimeMissingMessage(kind);
  equal(message.includes(RUNTIME_INSTALL_COMMANDS[kind]), true, `${kind} missing message should include its install command`);
  equal(message.includes("Locate binary"), true, `${kind} missing message should offer Locate binary`);
  equal(message.includes("on PATH."), false, `${kind} missing message should not be the bare PATH error`);
}
