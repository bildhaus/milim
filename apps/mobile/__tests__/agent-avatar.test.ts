import {
  AGENT_AVATAR_PALETTES,
  agentAvatarPalette,
  agentAvatarSeed,
  nativeAgentAvatarSvg,
} from '../src/agentAvatar';

describe('mobile Agent avatars', () => {
  test('resolves the same persisted seed fallbacks as desktop', () => {
    expect(agentAvatarSeed({name: 'Researcher', avatar: 'field-notes'})).toBe('field-notes');
    expect(agentAvatarSeed({name: 'Security', avatar: '🛡️'})).toBe('🛡️');
    expect(agentAvatarSeed({name: 'Researcher', avatar: '/images/legacy.png'})).toBe('Researcher');
    expect(agentAvatarSeed({id: 'agent-123'})).toBe('agent-123');
  });

  test('selects a deterministic desktop-compatible palette', () => {
    expect(agentAvatarPalette([0])).toBe(AGENT_AVATAR_PALETTES[0]);
    expect(agentAvatarPalette([0.999])).toBe(AGENT_AVATAR_PALETTES[7]);
  });

  test('removes unsupported native filters without discarding avatar geometry', () => {
    const svg = '<svg data-topology="2" style="display:block"><defs><filter id="sa-1-field"><feTurbulence /></filter></defs><g filter="url(#sa-1-field)"><path d="M0 0" /></g></svg>';
    const nativeSvg = nativeAgentAvatarSvg(svg);

    expect(nativeSvg).toContain('<path d="M0 0" />');
    expect(nativeSvg).not.toContain('<filter');
    expect(nativeSvg).not.toContain('<feTurbulence');
    expect(nativeSvg).not.toContain('filter="url(');
    expect(nativeSvg).not.toContain('data-topology');
    expect(nativeSvg).not.toContain('style=');
  });
});
