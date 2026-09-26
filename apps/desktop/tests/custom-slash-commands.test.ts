import type { CustomSlashCommand } from "../src/api.js";
import {
  customSlashCommandSourceLabel,
  mergeCustomSlashCommands,
  parseSlashCommandInput,
  resolveCustomSlashCommand,
  SLASH_COMMANDS,
} from "../src/lib/slashCommands.js";

function equal<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
}

function command(name: string, source: CustomSlashCommand["source"] = "project"): CustomSlashCommand {
  return { name, description: `${name} description`, argument_hint: null, source, path: `/tmp/${name}.md` };
}

const custom = [
  command("review"),
  command("model"),
  command("Git:Commit", "user"),
  command("review", "user"),
  command(""),
];

const merged = mergeCustomSlashCommands(custom);
equal(merged.map((entry) => entry.name).join(","), "review,git:commit", "built-ins win and duplicates keep the first definition");
equal(merged[0].source, "project", "project definition is kept over a later user duplicate");
equal(SLASH_COMMANDS.some((entry) => entry.id === "model"), true, "model stays a built-in");
equal(mergeCustomSlashCommands(custom, []).map((entry) => entry.name).join(","), "review,model,git:commit", "only listed built-ins shadow custom names");

const parsed = parseSlashCommandInput("  /git:commit fix the parser\nand tests  ");
equal(parsed?.id, "git:commit", "namespaced id is parsed");
equal(parsed?.argument, "fix the parser\nand tests", "multi-line arguments are kept");
equal(parseSlashCommandInput("/Review")?.argument, "", "no argument parses as empty");
equal(parseSlashCommandInput("please /review"), null, "commands must start the message");

const resolved = resolveCustomSlashCommand("/review src/main.rs", merged);
equal(resolved?.command.name, "review", "custom command resolves");
equal(resolved?.argument, "src/main.rs", "custom command argument resolves");
equal(resolveCustomSlashCommand("/GIT:COMMIT", merged)?.command.source, "user", "lookup is case-insensitive");
equal(resolveCustomSlashCommand("/model llama3.2", mergeCustomSlashCommands(custom, [])), null, "built-in names never resolve to custom commands");
equal(resolveCustomSlashCommand("/unknown", merged), null, "unknown commands do not resolve");

equal(customSlashCommandSourceLabel(merged[0]), "Project", "project label");
equal(customSlashCommandSourceLabel(merged[1]), "User", "user label");
