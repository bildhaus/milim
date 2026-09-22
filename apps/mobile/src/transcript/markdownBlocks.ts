type BlockToken = {level: number; nesting: number; map: [number, number] | null};
type BlockParser = {parse: (source: string, env: object) => BlockToken[]};

export type MarkdownBlockCache = {content: string; blocks: string[]};

// Reference-style link definitions resolve across the whole document, so a
// message that uses them cannot be rendered block by block.
const REFERENCE_DEFINITION = /^ {0,3}\[[^\]\n]+\]:\s/m;

// Splits Markdown into its top-level blocks. Joining the result always yields
// the input, so rendering the blocks in order is equivalent to rendering it whole.
export function splitMarkdownBlocks(parser: BlockParser, content: string): string[] {
  if (!content || REFERENCE_DEFINITION.test(content)) return [content];
  const starts: number[] = [];
  for (const token of parser.parse(content, {})) {
    if (token.level !== 0 || token.nesting < 0 || !token.map) continue;
    const line = token.map[0];
    if (line > 0 && line !== starts[starts.length - 1]) starts.push(line);
  }
  if (!starts.length) return [content];
  const lineOffsets = [0];
  for (let index = content.indexOf('\n'); index !== -1; index = content.indexOf('\n', index + 1)) {
    lineOffsets.push(index + 1);
  }
  const blocks: string[] = [];
  let from = 0;
  for (const line of starts) {
    const offset = lineOffsets[line];
    if (offset === undefined || offset <= from) continue;
    blocks.push(content.slice(from, offset));
    from = offset;
  }
  blocks.push(content.slice(from));
  return blocks;
}

// While an answer streams, text is only appended. Every block except the last
// is already closed, so only the tail is parsed again and the settled blocks
// keep their identity — which lets their rendered views be reused untouched.
export function splitMarkdownBlocksIncrementally(
  parser: BlockParser,
  previous: MarkdownBlockCache | null,
  content: string,
): MarkdownBlockCache {
  if (previous && previous.content === content) return previous;
  if (
    previous &&
    previous.blocks.length > 1 &&
    content.startsWith(previous.content) &&
    !REFERENCE_DEFINITION.test(content)
  ) {
    const settled = previous.blocks.slice(0, -1);
    const settledLength = settled.reduce((length, block) => length + block.length, 0);
    return {content, blocks: [...settled, ...splitMarkdownBlocks(parser, content.slice(settledLength))]};
  }
  return {content, blocks: splitMarkdownBlocks(parser, content)};
}
