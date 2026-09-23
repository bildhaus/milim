import React from 'react';
import {Modal, Pressable, StyleSheet, View} from 'react-native';
import {SafeAreaView} from 'react-native-safe-area-context';
import {MilimIcon} from './MilimIcon';
import {useAppTheme, Text} from './appTheme';
import {useReducedMotion, MotionPressable} from './motion';
import {SheetBackdropFade} from './fades';

export function PickerSheetFrame({
  visible,
  title,
  subtitle,
  onClose,
  children,
  compact = false,
}: {
  visible: boolean;
  title: string;
  subtitle: string;
  onClose: () => void;
  children: React.ReactNode;
  compact?: boolean;
}) {
  const {palette, styles} = useAppTheme();
  const reduced = useReducedMotion();
  return (
    <Modal
      visible={visible}
      transparent
      statusBarTranslucent
      animationType={reduced ? 'none' : 'slide'}
      onRequestClose={onClose}>
      <View style={styles.pickerModal}>
        <SheetBackdropFade />
        <Pressable style={StyleSheet.absoluteFill} onPress={onClose} accessibilityRole="button" accessibilityLabel={`Dismiss ${title}`} />
        <SafeAreaView style={[styles.pickerSheet, compact && styles.pickerSheetCompact]} edges={['bottom', 'left', 'right']} accessibilityViewIsModal>
          <View style={styles.sheetHandle} />
          <View style={styles.pickerHeader}>
            <View style={styles.flex}>
              <Text style={styles.pickerTitle} accessibilityRole="header">{title}</Text>
              <Text style={styles.pickerSubtitle}>{subtitle}</Text>
            </View>
            <MotionPressable style={styles.pickerClose} onPress={onClose} hitSlop={8} accessibilityLabel={`Close ${title}`}>
              <MilimIcon name="x" size={16} color={palette.secondary} />
            </MotionPressable>
          </View>
          {children}
        </SafeAreaView>
      </View>
    </Modal>
  );
}
