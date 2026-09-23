import React, {useEffect, useRef, useState} from 'react';
import {ScrollView, View, type StyleProp, type TextStyle} from 'react-native';
import Clipboard from '@react-native-clipboard/clipboard';
import {MilimIcon} from '../ui/MilimIcon';
import {useAppTheme, Text} from '../ui/appTheme';
import {MotionPressable} from '../ui/motion';
import {haptics} from '../ui/haptics';

export function CodeBlock({content, language, textStyle}: {content: string; language?: string; textStyle: StyleProp<TextStyle>}) {
  const {palette, styles} = useAppTheme();
  const [copied, setCopied] = useState(false);
  const resetTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => {
    if (resetTimer.current) clearTimeout(resetTimer.current);
  }, []);
  const copy = () => {
    Clipboard.setString(content);
    haptics.success();
    setCopied(true);
    if (resetTimer.current) clearTimeout(resetTimer.current);
    resetTimer.current = setTimeout(() => setCopied(false), 1600);
  };
  return (
    <View style={styles.markdownCode}>
      <View style={styles.markdownCodeHeader}>
        <Text style={styles.markdownCodeLanguage} numberOfLines={1}>{language || 'code'}</Text>
        <MotionPressable
          style={styles.markdownCodeCopy}
          hitSlop={8}
          onPress={copy}
          accessibilityLabel={copied ? 'Code copied' : `Copy ${language || 'code'}`}>
          <MilimIcon name={copied ? 'check' : 'copy'} size={12} color={copied ? palette.success : palette.muted} />
          <Text style={styles.markdownCodeCopyText}>{copied ? 'Copied' : 'Copy'}</Text>
        </MotionPressable>
      </View>
      <ScrollView horizontal nestedScrollEnabled showsHorizontalScrollIndicator>
        <Text style={[textStyle, styles.markdownCodeText]} selectable>{content}</Text>
      </ScrollView>
    </View>
  );
}
