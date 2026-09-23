import {ScrollView, View} from 'react-native';
import {MarkdownIt, type RenderRules} from 'react-native-markdown-display';
import {MOBILE_MARKDOWN_OPTIONS} from '../markdown';
import {CodeBlock} from './CodeBlock';
import {TranscriptImage} from './TranscriptImage';

export function markdownCodeContent(content: string): string {
  return content.endsWith('\n') ? content.slice(0, -1) : content;
}

export const mobileMarkdownRules: RenderRules = {
  code_block: (node, _children, _parents, styles, inheritedStyles = {}) => (
    <CodeBlock
      key={node.key}
      content={markdownCodeContent(node.content)}
      textStyle={[inheritedStyles, styles.code_block]}
    />
  ),
  fence: (node, _children, _parents, styles, inheritedStyles = {}) => (
    <CodeBlock
      key={node.key}
      content={markdownCodeContent(node.content)}
      language={String((node as {sourceInfo?: string}).sourceInfo ?? '').trim().split(/\s+/)[0]}
      textStyle={[inheritedStyles, styles.fence]}
    />
  ),
  table: (node, children, _parents, styles) => (
    <ScrollView key={node.key} horizontal nestedScrollEnabled showsHorizontalScrollIndicator>
      <View style={styles._VIEW_SAFE_table}>{children}</View>
    </ScrollView>
  ),
  image: node => (
    <TranscriptImage
      key={node.key}
      source={String(node.attributes.src ?? '')}
      alt={String(node.attributes.alt ?? '')}
    />
  ),
};

export const mobileMarkdownParser = new MarkdownIt(MOBILE_MARKDOWN_OPTIONS);
