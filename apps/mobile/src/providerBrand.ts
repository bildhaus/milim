export type ProviderBrand =
  | 'openai'
  | 'openrouter'
  | 'groq'
  | 'claude'
  | 'gemini'
  | 'replicate'
  | 'fal'
  | 'ollama'
  | 'lmstudio'
  | 'codex'
  | 'opencode'
  | 'pi';

export const PROVIDER_BRANDS: ProviderBrand[] = [
  'openai',
  'openrouter',
  'groq',
  'claude',
  'gemini',
  'replicate',
  'fal',
  'ollama',
  'lmstudio',
  'codex',
  'opencode',
  'pi',
];

const NAMES: Record<string, ProviderBrand> = {
  openai: 'openai',
  openrouter: 'openrouter',
  openroutermedia: 'openrouter',
  groq: 'groq',
  anthropic: 'claude',
  claude: 'claude',
  localclaudecli: 'claude',
  gemini: 'gemini',
  google: 'gemini',
  replicate: 'replicate',
  replicatemedia: 'replicate',
  fal: 'fal',
  falmedia: 'fal',
  ollama: 'ollama',
  ollamalocal: 'ollama',
  lmstudio: 'lmstudio',
  lmstudiolocal: 'lmstudio',
  codex: 'codex',
  opencode: 'opencode',
  localopencodecli: 'opencode',
  pi: 'pi',
  localpicli: 'pi',
};

export function providerBrandForModel({
  id,
  owner,
  provider,
}: {
  id: string;
  owner?: string;
  provider?: string;
}): ProviderBrand | null {
  const normalizedId = id.toLowerCase();
  if (normalizedId.startsWith('codex:')) return 'codex';
  if (normalizedId.startsWith('claude:')) return 'claude';
  if (normalizedId.startsWith('opencode:')) return 'opencode';
  if (normalizedId.startsWith('pi:')) return 'pi';
  for (const candidate of [owner, provider]) {
    const normalized = candidate?.toLowerCase().replace(/[^a-z0-9]+/g, '') ?? '';
    if (NAMES[normalized]) return NAMES[normalized];
  }
  return null;
}
