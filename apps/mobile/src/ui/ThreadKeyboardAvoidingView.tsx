import React from 'react';
import {KeyboardAvoidingView} from 'react-native-keyboard-controller';
import {useAppTheme} from './appTheme';

// Follows the keyboard frame by frame on both platforms, including an
// interactive swipe-to-dismiss, instead of animating to its final height.
export function ThreadKeyboardAvoidingView({
  children,
  enabled,
}: {
  children: React.ReactNode;
  enabled: boolean;
}) {
  const {styles} = useAppTheme();
  return (
    <KeyboardAvoidingView style={styles.content} behavior="padding" enabled={enabled}>
      {children}
    </KeyboardAvoidingView>
  );
}
