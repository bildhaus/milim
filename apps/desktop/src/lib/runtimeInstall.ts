import type { AccountRuntimeKind } from "../api";

/**
 * Official install commands for the account-runtime CLIs. These are the same
 * npm packages the Providers update check compares against; keep them in
 * sync with docs/account-runtimes.md.
 */
export const RUNTIME_INSTALL_COMMANDS: Record<AccountRuntimeKind, string> = {
  codex: "npm install -g @openai/codex",
  claude: "npm install -g @anthropic-ai/claude-code",
  opencode: "npm install -g opencode-ai",
  pi: "npm install -g @earendil-works/pi-coding-agent",
};

export const RUNTIME_CLI_LABELS: Record<AccountRuntimeKind, string> = {
  codex: "Codex",
  claude: "Claude",
  opencode: "OpenCode",
  pi: "Pi",
};

/** Replacement for a bare "CLI not found on PATH." */
export function runtimeMissingMessage(kind: AccountRuntimeKind): string {
  return `${RUNTIME_CLI_LABELS[kind]} CLI not found. Install it with \`${RUNTIME_INSTALL_COMMANDS[kind]}\`, or use Locate binary... to choose the executable.`;
}
