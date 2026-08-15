import {modelPickerGroups, parseMobileModel} from '../src/modelPicker';

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
  });
  expect(model?.capabilities).toEqual(expect.arrayContaining(['vision', 'tools', 'reasoning', 'fast']));
});

test('searches across labels, ids, providers, and routes', () => {
  const values = [
    {id: 'codex:gpt-5.3-codex', owned_by: 'openai'},
    {id: 'claude:opus', owned_by: 'anthropic'},
  ];
  expect(modelPickerGroups(values, 'codex')).toHaveLength(1);
  expect(modelPickerGroups(values, 'anthropic')[0].models[0].id).toBe('claude:opus');
});
