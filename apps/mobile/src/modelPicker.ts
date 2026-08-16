import type {JsonValue} from './control/types';

export type MobileModelCapability =
  | 'vision'
  | 'tools'
  | 'reasoning'
  | 'fast'
  | 'image'
  | 'video'
  | 'music';

export interface MobileModelOption {
  id: string;
  label: string;
  provider: string;
  route: string;
  owner: string;
  detail: string | null;
  capabilities: MobileModelCapability[];
  reasoningEfforts: string[];
}

function record(value: JsonValue | undefined): Record<string, JsonValue> | null {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as Record<string, JsonValue>)
    : null;
}

function text(value: JsonValue | undefined): string {
  return typeof value === 'string' ? value : '';
}

function bool(object: Record<string, JsonValue> | null, ...keys: string[]): boolean | undefined {
  for (const key of keys) {
    if (typeof object?.[key] === 'boolean') return object[key] as boolean;
  }
  return undefined;
}

function titleCase(value: string): string {
  const acronyms: Record<string, string> = {
    ai: 'AI',
    api: 'API',
    github: 'GitHub',
    lmstudio: 'LM Studio',
    openai: 'OpenAI',
    openrouter: 'OpenRouter',
  };
  return value
    .split(/[-_.]+/)
    .filter(Boolean)
    .map(part => acronyms[part.toLowerCase()] ?? `${part[0]?.toUpperCase() ?? ''}${part.slice(1)}`)
    .join(' ');
}

function compactTokens(value: number): string {
  if (value >= 1_000_000) return `${Number((value / 1_000_000).toFixed(1))}M`;
  if (value >= 1_000) return `${Number((value / 1_000).toFixed(1))}k`;
  return String(value);
}

export function parseMobileModel(value: JsonValue): MobileModelOption | null {
  if (typeof value === 'string') {
    return {
      id: value,
      label: value,
      provider: 'Models',
      route: 'Models',
      owner: '',
      detail: null,
      capabilities: [],
      reasoningEfforts: [],
    };
  }
  const model = record(value);
  const id = text(model?.id).trim();
  if (!id) return null;
  const owner = text(model?.owned_by).trim();
  const capabilities = record(model?.capabilities);
  const reasoning = record(model?.reasoning);
  const lower = id.toLowerCase();
  const ownerLower = owner.toLowerCase();
  const output: MobileModelCapability[] = [];
  const imageInput = bool(capabilities, 'image_input', 'imageInput');
  if (
    imageInput === true ||
    (imageInput === undefined && /(vision|llava|pixtral|llama-4|qwen[^/]*vl|-vl)/.test(lower))
  ) output.push('vision');
  if (bool(capabilities, 'tool_use', 'toolUse')) output.push('tools');
  if (bool(capabilities, 'image_output', 'imageOutput')) output.push('image');
  if (bool(capabilities, 'video_output', 'videoOutput')) output.push('video');
  if (bool(capabilities, 'music_output', 'musicOutput')) output.push('music');
  if (reasoning || /(r1|reason|qwq|o1|o3|-think|deepseek-r)/.test(lower)) output.push('reasoning');
  if (/(flash|mini|haiku|turbo|instant|nano|small)/.test(lower)) output.push('fast');

  let provider = titleCase(owner) || 'Models';
  let route = provider;
  let label = text(model?.display_id) || id;
  if (id.startsWith('codex:')) {
    provider = route = 'Codex';
    label = id.slice('codex:'.length);
  } else if (id.startsWith('claude:')) {
    provider = route = 'Claude CLI';
    label = id.slice('claude:'.length);
  } else if (id.startsWith('opencode:') || id.startsWith('pi:')) {
    const runtime = id.startsWith('opencode:') ? 'OpenCode' : 'Pi';
    const routed = id.slice(id.indexOf(':') + 1);
    const nested = routed.split('/')[0];
    provider = runtime;
    route = nested ? `${runtime} · ${titleCase(nested)}` : runtime;
    label = routed.slice(routed.indexOf('/') + 1) || routed;
  } else if (!owner && ownerLower !== 'milim') {
    provider = route = 'Local models';
  }
  const context = Number(model?.context_length ?? model?.max_prompt_tokens ?? 0);
  const detail = Number.isFinite(context) && context > 0 ? `${compactTokens(context)} context` : null;
  const efforts = Array.isArray(reasoning?.supported_efforts)
    ? reasoning.supported_efforts.filter((effort): effort is string => typeof effort === 'string')
    : [];
  return {
    id,
    label,
    provider,
    route,
    owner,
    detail,
    capabilities: [...new Set(output)],
    reasoningEfforts: efforts,
  };
}

export function modelPickerGroups(
  values: JsonValue[],
  query: string,
): Array<{title: string; models: MobileModelOption[]}> {
  const normalized = query.trim().toLowerCase();
  const groups = new Map<string, MobileModelOption[]>();
  for (const value of values) {
    const model = parseMobileModel(value);
    if (!model) continue;
    if (
      normalized &&
      ![model.label, model.id, model.provider, model.route, model.owner].some(part =>
        part.toLowerCase().includes(normalized),
      )
    ) continue;
    groups.set(model.provider, [...(groups.get(model.provider) ?? []), model]);
  }
  return Array.from(groups, ([title, models], index) => ({title, models, index}))
    .sort((left, right) => left.models.length - right.models.length || left.index - right.index)
    .map(({title, models}) => ({title, models}));
}
