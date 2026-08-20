import {
  modelPickerFavoriteIds,
  modelPickerGroups,
  normalizeModelPickerPreferences,
  parseMobileModel,
  transcriptModelLabel,
  toggledModelFavoriteIds,
} from '../src/modelPicker';

test('projects runtime model labels, provider routes, and capabilities', () => {
  const model = parseMobileModel({
    id: 'opencode:openai/gpt-5-mini',
    owned_by: 'opencode',
    context_length: 128000,
    capabilities: {image_input: true, tool_use: true},
    reasoning: {supported_efforts: ['low', 'high']},
  });
  expect(model).toMatchObject({
    label: 'gpt-5-mini',
    provider: 'OpenCode',
    route: 'OpenCode · OpenAI',
    detail: '128k context',
    reasoningEfforts: ['low', 'high'],
    brand: 'opencode',
  });
  expect(model?.capabilities).toEqual(expect.arrayContaining(['vision', 'tools', 'reasoning', 'fast']));
});

test('matches desktop provider and runtime brands', () => {
  expect(parseMobileModel({id: 'gpt-5', owned_by: 'OpenAI'})?.brand).toBe('openai');
  expect(parseMobileModel({id: 'gemini-3-pro', owned_by: 'Gemini'})?.brand).toBe('gemini');
  expect(parseMobileModel({id: 'claude:opus', owned_by: 'milim'})?.brand).toBe('claude');
  expect(parseMobileModel({id: 'local-model', owned_by: 'LM Studio Local'})?.brand).toBe('lmstudio');
});

test('does not present the local service placeholder as a Milim provider', () => {
  const google = parseMobileModel({id: 'google/gemma-4-26b', owned_by: 'milim'});
  expect(google).toMatchObject({
    provider: 'Local models',
    route: 'Local models · Google',
    brand: 'gemini',
  });
  expect(parseMobileModel({id: 'black-forest-labs/flux.2', owned_by: 'milim'})).toMatchObject({
    provider: 'Local models',
    route: 'Local models · Black Forest Labs',
    brand: null,
  });
});

test('adds favorites first and supports favorites-only filtering', () => {
  const values = [
    {id: 'codex:gpt-5.3-codex', owned_by: 'openai'},
    {id: 'claude:opus', owned_by: 'anthropic'},
  ];
  const groups = modelPickerGroups(values, '', ['claude:opus']);
  expect(groups[0].title).toBe('Favorites');
  expect(groups[0].models.map(model => model.id)).toEqual(['claude:opus']);
  expect(modelPickerGroups(values, '', ['claude:opus'], true)).toHaveLength(1);
});

test('normalizes persisted mobile picker preferences', () => {
  expect(normalizeModelPickerPreferences({
    favorites: ['a', 'a', 42],
    favoritesOnly: 'yes',
    collapsedGroups: ['OpenAI', null],
  })).toEqual({favorites: ['a'], favoritesOnly: false, collapsedGroups: ['OpenAI']});
});

test('uses host favorites when supported and keeps local favorites as a legacy fallback', () => {
  expect(modelPickerFavoriteIds([' desktop:model ', 'desktop:model'], ['mobile:model']))
    .toEqual(['desktop:model']);
  expect(modelPickerFavoriteIds(undefined, ['mobile:model'])).toEqual(['mobile:model']);
  expect(toggledModelFavoriteIds(['desktop:model'], 'provider:model'))
    .toEqual(['desktop:model', 'provider:model']);
});

test('searches across labels, ids, providers, and routes', () => {
  const values = [
    {id: 'codex:gpt-5.3-codex', owned_by: 'openai'},
    {id: 'claude:opus', owned_by: 'anthropic'},
  ];
  expect(modelPickerGroups(values, 'codex')).toHaveLength(1);
  expect(modelPickerGroups(values, 'anthropic')[0].models[0].id).toBe('claude:opus');
});

test('formats routed model ids for transcript notices', () => {
  expect(transcriptModelLabel('provider:remote-1:openai/gpt-5.6')).toBe('openai/gpt-5.6');
  expect(transcriptModelLabel('codex:gpt-5.6-luna')).toBe('gpt-5.6-luna');
  expect(transcriptModelLabel('opencode:openai/gpt-5.6')).toBe('gpt-5.6');
});
