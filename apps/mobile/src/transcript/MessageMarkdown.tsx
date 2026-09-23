import React, {useRef} from 'react';
import Markdown from 'react-native-markdown-display';
import type {useAppTheme} from '../ui/appTheme';
import {splitMarkdownBlocksIncrementally, type MarkdownBlockCache} from './markdownBlocks';
import {mobileMarkdownParser, mobileMarkdownRules} from './markdownRules';

type MarkdownStyles = ReturnType<typeof useAppTheme>['markdownStyles'];

// One top-level Markdown block. Its text is its identity: while an answer
// streams, every settled block keeps the same text and skips rendering.
const MarkdownBlock = React.memo(function MemoizedMarkdownBlock({text, style}: {text: string; style: MarkdownStyles}) {
  return (
    <Markdown markdownit={mobileMarkdownParser} style={style} rules={mobileMarkdownRules}>
      {text}
    </Markdown>
  );
});

// Renders a message block by block so a streamed token re-parses and re-renders
// only the block still being written, not the whole answer.
export function MessageMarkdown({content, style}: {content: string; style: MarkdownStyles}) {
  const cache = useRef<MarkdownBlockCache | null>(null);
  cache.current = splitMarkdownBlocksIncrementally(mobileMarkdownParser, cache.current, content);
  return (
    <>
      {cache.current.blocks.map((text, index) => (
        <MarkdownBlock key={index} text={text} style={style} />
      ))}
    </>
  );
}
