import {createRequire} from 'node:module';
import {MOBILE_MARKDOWN_OPTIONS} from '../src/markdown';

type MarkdownToken = {
  type: string;
  attrGet(name: string): string | null;
};

const rendererRequire = createRequire(
  require.resolve('react-native-markdown-display/package.json'),
);
const MarkdownIt = rendererRequire('markdown-it');

describe('mobile transcript Markdown', () => {
  test('turns a pasted user URL into a tappable link token', () => {
    const parser = new MarkdownIt(MOBILE_MARKDOWN_OPTIONS);
    const inline = parser.parseInline('Open https://milim.ai/docs', {})[0];
    const link = (inline.children as MarkdownToken[]).find(token => token.type === 'link_open');

    expect(link?.attrGet('href')).toBe('https://milim.ai/docs');
  });
});
