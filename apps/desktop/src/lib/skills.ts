import type { ChatMessage, SkillInfo } from "../api";

const MAX_SKILL_CHARS = 12_000;
const MAX_SKILL_DESCRIPTION_CHARS = 220;

export function skillInstructionMessage(skills: SkillInfo[]): ChatMessage | null {
  const enabled = skills.filter((s) => s.enabled);
  if (!enabled.length) return null;
  const blocks: string[] = [];
  for (const skill of enabled) {
    const block = fullSkillBlock(skill, blocks.length + 1);
    if (blocks.length > 0 && [...blocks, block].join("\n\n").length > MAX_SKILL_CHARS) {
      continue;
    }
    blocks.push(block);
  }
  const omitted = enabled.length - blocks.length;
  const body = [
    blocks.join("\n\n"),
    omitted > 0 ? `[${omitted} additional skill${omitted === 1 ? "" : "s"} omitted by the prompt budget]` : "",
  ].filter(Boolean).join("\n\n");
  return {
    role: "system",
    content: `Use these installed skills when relevant. Follow their instructions only if they help with the user's current request.\n\n${body}`,
  };
}

export function skillDiscoveryMessage(
  skills: SkillInfo[],
  explicitSkillIds: string[],
): ChatMessage | null {
  const explicit = new Set(explicitSkillIds);
  const loaded = skills.filter((skill) => skill.enabled && explicit.has(skill.id));
  const candidates = skills.filter((skill) => skill.enabled && !explicit.has(skill.id));
  if (!loaded.length && !candidates.length) return null;
  const loadedMessage = skillInstructionMessage(loaded)?.content ?? "";
  const catalog = candidates.map((skill) => {
    const description = compactDescription(skill.description);
    return `- ${skill.name} (id: ${skill.id})${description ? `: ${description}` : ""}`;
  }).join("\n");
  return {
    role: "system",
    content: [
      loadedMessage,
      catalog ? [
        "Relevant Milim skills available for this turn:",
        catalog,
        "Skill bodies are not in context. Before following one, call milim_skill_read with its id. Use milim_skill_search if these candidates are insufficient.",
      ].join("\n") : "",
    ].filter(Boolean).join("\n\n"),
  };
}

function fullSkillBlock(skill: SkillInfo, index: number): string {
  const desc = skill.description.trim();
  return [
    `## ${index}. ${skill.name}`,
    desc ? `Description: ${desc}` : "",
    skill.instructions.trim(),
  ].filter(Boolean).join("\n");
}

function compactDescription(description: string): string {
  const normalized = description.trim().replace(/\s+/g, " ");
  if (normalized.length <= MAX_SKILL_DESCRIPTION_CHARS) return normalized;
  const shortened = normalized.slice(0, MAX_SKILL_DESCRIPTION_CHARS + 1);
  const boundary = shortened.lastIndexOf(" ");
  return `${shortened.slice(0, boundary > 0 ? boundary : MAX_SKILL_DESCRIPTION_CHARS).trimEnd()}...`;
}
