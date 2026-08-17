export interface AgentAvatarIdentity {
  id?: string;
  name?: string;
  avatar?: string;
}

export const AGENT_AVATAR_PALETTES = [
  {color: '#ff6b6b', secondaryColor: '#28b8b4', background: '#ffe3bf'},
  {color: '#7c5cff', secondaryColor: '#e84a9b', background: '#e9ddff'},
  {color: '#198754', secondaryColor: '#e0a800', background: '#d9f4df'},
  {color: '#0f62fe', secondaryColor: '#33b1ff', background: '#d8e8ff'},
  {color: '#9f1853', secondaryColor: '#fa4d56', background: '#ffd6e8'},
  {color: '#007d79', secondaryColor: '#42be65', background: '#d1f5f2'},
  {color: '#6929c4', secondaryColor: '#1192e8', background: '#e8daff'},
  {color: '#b28600', secondaryColor: '#ff832b', background: '#fff1c7'},
] as const;

function isLegacyAgentAvatar(value: string): boolean {
  return (
    value.startsWith('data:') ||
    value.startsWith('/images/') ||
    /\.(png|jpe?g|webp|gif)$/i.test(value)
  );
}

export function agentAvatarSeed(agent: AgentAvatarIdentity): string {
  const raw = (agent.avatar ?? '').trim();
  if (raw && !isLegacyAgentAvatar(raw)) return raw;
  const name = (agent.name ?? '').trim();
  return name || (agent.id ?? '').trim();
}

export function agentAvatarPalette(recipeShape: number[]) {
  const index = Math.floor(recipeShape[0] * AGENT_AVATAR_PALETTES.length);
  return AGENT_AVATAR_PALETTES[index];
}

/** Remove web-only SVG filters while preserving the package's native-safe geometry and gradients. */
export function nativeAgentAvatarSvg(svg: string): string {
  return svg
    .replace(/<filter id="[^"]+-field"[\s\S]*?<\/filter>/, '')
    .replace(/\sfilter="url\(#[^"]+-field\)"/g, '')
    .replace(/\sdata-[a-z-]+="[^"]*"/g, '')
    .replace(/\sstyle="[^"]*"/g, '');
}
