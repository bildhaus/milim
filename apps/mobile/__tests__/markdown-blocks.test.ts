import {createRequire} from 'node:module';
import {MOBILE_MARKDOWN_OPTIONS} from '../src/markdown';
import {splitMarkdownBlocks, splitMarkdownBlocksIncrementally, type MarkdownBlockCache} from '../src/transcript/markdownBlocks';

const rendererRequire = createRequire(
  require.resolve('react-native-markdown-display/package.json'),
);
const MarkdownIt = rendererRequire('markdown-it');
const parser = new MarkdownIt(MOBILE_MARKDOWN_OPTIONS);

const answer = [
  '# Plan',
  '',
  'First paragraph with `code`.',
  '',
  '- one',
  '- two',
  '',
  '  still item two',
  '',
  '```ts',
  'const a = 1;',
  '',
  'const b = 2;',
  '```',
  '',
  '| a | b |',
  '|---|---|',
  '| 1 | 2 |',
  '',
  'Done.',
  '',
].join('\n');

test('blocks are the top-level Markdown elements and rejoin to the source', () => {
  const blocks = splitMarkdownBlocks(parser, answer);
  expect(blocks.join('')).toBe(answer);
  expect(blocks.map(block => block.trim().split('\n')[0])).toEqual([
    '# Plan',
    'First paragraph with `code`.',
    '- one',
    '```ts',
    '| a | b |',
    'Done.',
  ]);
});

test('a blank line inside a fence or list item does not split the block', () => {
  const blocks = splitMarkdownBlocks(parser, answer);
  expect(blocks.find(block => block.startsWith('```ts'))).toContain('const b = 2;');
  expect(blocks.find(block => block.startsWith('- one'))).toContain('still item two');
});

test('streaming reuses settled blocks and only re-splits the tail', () => {
  let cache: MarkdownBlockCache | null = null;
  let previousBlocks: string[] = [];
  for (let end = 1; end <= answer.length; end += 1) {
    const content = answer.slice(0, end);
    cache = splitMarkdownBlocksIncrementally(parser, cache, content);
    expect(cache.blocks.join('')).toBe(content);
    // Every block before the last one is carried over unchanged.
    const carried = Math.min(previousBlocks.length, cache.blocks.length) - 1;
    for (let index = 0; index < carried; index += 1) {
      expect(cache.blocks[index]).toBe(previousBlocks[index]);
    }
    previousBlocks = cache.blocks;
  }
  expect(cache?.blocks).toEqual(splitMarkdownBlocks(parser, answer));
});

test('an unchanged message returns the same cache object', () => {
  const first = splitMarkdownBlocksIncrementally(parser, null, answer);
  expect(splitMarkdownBlocksIncrementally(parser, first, answer)).toBe(first);
});

test('edited or replaced content is split from scratch', () => {
  const first = splitMarkdownBlocksIncrementally(parser, null, answer);
  const replaced = splitMarkdownBlocksIncrementally(parser, first, 'Different.\n\nText.');
  expect(replaced.blocks).toEqual(['Different.\n\n', 'Text.']);
});

test('reference-style links keep the message whole so they still resolve', () => {
  const content = 'See [docs][d].\n\nMore text.\n\n[d]: https://milim.ai/docs\n';
  expect(splitMarkdownBlocks(parser, content)).toEqual([content]);
  const streamed = splitMarkdownBlocksIncrementally(parser, {content: 'See [docs][d].\n\nMore text.\n\n', blocks: ['See [docs][d].\n\n', 'More text.\n\n']}, content);
  expect(streamed.blocks).toEqual([content]);
});
