import React from 'react';
import {act, create} from 'react-test-renderer';
import {createHotStore, useHotState} from '../src/controller/hotStore';
import {MessageMarkdown} from '../src/transcript/MessageMarkdown';

const mockBlockRenders: string[] = [];

jest.mock('react-native-markdown-display', () => ({
  __esModule: true,
  default: ({children}: {children: string}) => {
    mockBlockRenders.push(children);
    return null;
  },
}));
jest.mock('../src/transcript/markdownRules', () => {
  const {createRequire} = require('node:module');
  const rendererRequire = createRequire(require.resolve('react-native-markdown-display/package.json'));
  const MarkdownIt = rendererRequire('markdown-it');
  return {mobileMarkdownParser: new MarkdownIt({linkify: true, typographer: true}), mobileMarkdownRules: {}};
});

let consoleError: jest.SpyInstance;
beforeEach(() => {
  Object.assign(globalThis, {IS_REACT_ACT_ENVIRONMENT: true});
  mockBlockRenders.length = 0;
  const originalError = console.error;
  consoleError = jest.spyOn(console, 'error').mockImplementation((message, ...args) => {
    if (String(message).startsWith('react-test-renderer is deprecated.')) return;
    originalError(message, ...args);
  });
});
afterEach(() => consoleError.mockRestore());

test('a streamed token re-renders only the Markdown block still being written', () => {
  const style = {} as never;
  const settled = '# Title\n\nFirst paragraph.\n\n';
  let renderer!: ReturnType<typeof create>;
  act(() => {
    renderer = create(<MessageMarkdown content={`${settled}Second par`} style={style} />);
  });
  expect(mockBlockRenders).toEqual(['# Title\n\n', 'First paragraph.\n\n', 'Second par']);

  mockBlockRenders.length = 0;
  act(() => renderer.update(<MessageMarkdown content={`${settled}Second paragraph grows`} style={style} />));
  expect(mockBlockRenders).toEqual(['Second paragraph grows']);

  mockBlockRenders.length = 0;
  act(() => renderer.update(<MessageMarkdown content={`${settled}Second paragraph grows.\n\nThird`} style={style} />));
  expect(mockBlockRenders).toEqual(['Second paragraph grows.\n\n', 'Third']);
});

test('hot state updates reach only the components that select them', () => {
  const store = createHotStore();
  const renders = {timeline: 0, draft: 0, hasDraft: 0, idle: 0};
  function TimelineReader() {
    useHotState(store, state => state.timeline);
    renders.timeline += 1;
    return null;
  }
  function DraftReader() {
    useHotState(store, state => state.draft);
    renders.draft += 1;
    return null;
  }
  function HasDraftReader() {
    useHotState(store, state => state.draft.trim().length > 0);
    renders.hasDraft += 1;
    return null;
  }
  const Idle = React.memo(function IdleShell() {
    renders.idle += 1;
    return null;
  });
  act(() => {
    create(<><TimelineReader /><DraftReader /><HasDraftReader /><Idle /></>);
  });
  expect(renders).toEqual({timeline: 1, draft: 1, hasDraft: 1, idle: 1});

  act(() => store.set({draft: 'h'}));
  act(() => store.set({draft: 'he'}));
  act(() => store.set({draft: 'hey'}));
  // Typing re-renders the draft reader per keystroke, the emptiness reader once.
  expect(renders).toEqual({timeline: 1, draft: 4, hasDraft: 2, idle: 1});

  const timeline = {threadId: 't', epoch: 'e', items: [], hasOlder: false} as never;
  act(() => store.set({timeline}));
  expect(renders).toEqual({timeline: 2, draft: 4, hasDraft: 2, idle: 1});

  // Writing an identical value notifies nobody.
  act(() => store.set({timeline, draft: 'hey'}));
  expect(renders).toEqual({timeline: 2, draft: 4, hasDraft: 2, idle: 1});
});
