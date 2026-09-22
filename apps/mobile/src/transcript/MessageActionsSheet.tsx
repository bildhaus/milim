import React, {useEffect, useState} from 'react';
import {Platform, ScrollView, Share, TextInput, View} from 'react-native';
import Clipboard from '@react-native-clipboard/clipboard';
import {useAppTheme, Text} from '../ui/appTheme';
import {ActionSheetButton} from '../ui/controls';
import {showError} from '../ui/dialogs';
import {haptics} from '../ui/haptics';
import {PickerSheetFrame} from '../ui/PickerSheetFrame';

export type MessageActionTarget = {role: 'user' | 'assistant' | 'system'; content: string};

const ROLE_TITLES: Record<MessageActionTarget['role'], string> = {
  user: 'Your message',
  assistant: 'milim’s answer',
  system: 'System message',
};

// One sheet for the whole transcript, opened by long-pressing a message.
export function MessageActionsSheet({target, onClose}: {target: MessageActionTarget | null; onClose: () => void}) {
  const {styles} = useAppTheme();
  const [selecting, setSelecting] = useState(false);
  // Keeps the last message on screen while the sheet slides away.
  const [shown, setShown] = useState<MessageActionTarget | null>(target);
  useEffect(() => {
    if (target) {
      setShown(target);
      setSelecting(false);
    }
  }, [target]);
  const content = shown?.content ?? '';
  const words = content.trim() ? content.trim().split(/\s+/).length : 0;

  return (
    <PickerSheetFrame
      visible={Boolean(target)}
      title={selecting ? 'Select text' : ROLE_TITLES[shown?.role ?? 'assistant']}
      subtitle={`${words} ${words === 1 ? 'word' : 'words'}`}
      compact={!selecting}
      onClose={onClose}>
      {selecting ? (
        <ScrollView style={styles.selectTextBody} contentContainerStyle={styles.selectTextContent}>
          {Platform.OS === 'ios' ? (
            // A read-only text view is what gives iOS word-level selection handles.
            <TextInput style={styles.selectTextValue} value={content} editable={false} multiline scrollEnabled={false} />
          ) : (
            <Text style={styles.selectTextValue} selectable>{content}</Text>
          )}
        </ScrollView>
      ) : (
        <View style={styles.actionSheetList}>
          <ActionSheetButton icon="copy" label="Copy" onPress={() => {
            Clipboard.setString(content);
            haptics.success();
            onClose();
          }} />
          <ActionSheetButton icon="text-select" label="Select text" onPress={() => setSelecting(true)} />
          <ActionSheetButton icon="share" label="Share…" onPress={() => {
            onClose();
            void Share.share({message: content}).catch(showError);
          }} />
        </View>
      )}
    </PickerSheetFrame>
  );
}
