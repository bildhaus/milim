import React, {useEffect, useRef, useSyncExternalStore} from 'react';
import {AccessibilityInfo, Animated, Pressable, StyleSheet, type StyleProp, type ViewStyle} from 'react-native';

// Every animated control asks for this, so the OS setting is read once and
// shared instead of opening a native subscription per button.
let reducedMotion = false;
let reducedMotionStarted = false;
const reducedMotionListeners = new Set<() => void>();

function setReducedMotion(value: boolean) {
  if (reducedMotion === value) return;
  reducedMotion = value;
  for (const listener of [...reducedMotionListeners]) listener();
}

function subscribeReducedMotion(listener: () => void) {
  reducedMotionListeners.add(listener);
  if (!reducedMotionStarted) {
    reducedMotionStarted = true;
    void AccessibilityInfo.isReduceMotionEnabled().then(setReducedMotion);
    AccessibilityInfo.addEventListener('reduceMotionChanged', setReducedMotion);
  }
  return () => {
    reducedMotionListeners.delete(listener);
  };
}

export function useReducedMotion(): boolean {
  return useSyncExternalStore(subscribeReducedMotion, () => reducedMotion);
}

export const AnimatedPressable = Animated.createAnimatedComponent(Pressable);

export function MotionPressable({
  children,
  style,
  onPress,
  disabled,
  hitSlop,
  accessibilityLabel,
  accessibilityRole = 'button',
  accessibilityState,
}: {
  children: React.ReactNode;
  style?: StyleProp<ViewStyle>;
  onPress: () => void;
  disabled?: boolean;
  hitSlop?: number;
  accessibilityLabel?: string;
  accessibilityRole?: React.ComponentProps<typeof Pressable>['accessibilityRole'];
  accessibilityState?: React.ComponentProps<typeof Pressable>['accessibilityState'];
}) {
  const reduced = useReducedMotion();
  const scale = useRef(new Animated.Value(1)).current;
  const animate = (value: number, duration: number) => {
    if (reduced) {
      scale.setValue(value);
      return;
    }
    Animated.timing(scale, {
      toValue: value,
      duration,
      useNativeDriver: true,
    }).start();
  };
  return (
    <AnimatedPressable
      style={[style, {transform: [{scale}]}]}
      onPress={onPress}
      disabled={disabled}
      hitSlop={hitSlop}
      accessibilityRole={accessibilityRole}
      accessibilityLabel={accessibilityLabel}
      accessibilityState={accessibilityState}
      onPressIn={() => animate(0.97, 100)}
      onPressOut={() => animate(1, 140)}>
      {children}
    </AnimatedPressable>
  );
}

export function ScreenStage({children}: {children: React.ReactNode}) {
  const reduced = useReducedMotion();
  const opacity = useRef(new Animated.Value(reduced ? 1 : 0)).current;
  const translateY = useRef(new Animated.Value(reduced ? 0 : 5)).current;
  useEffect(() => {
    if (reduced) {
      opacity.setValue(1);
      translateY.setValue(0);
      return;
    }
    Animated.parallel([
      Animated.timing(opacity, {toValue: 1, duration: 180, useNativeDriver: true}),
      Animated.timing(translateY, {toValue: 0, duration: 180, useNativeDriver: true}),
    ]).start();
  }, [opacity, reduced, translateY]);
  return <Animated.View style={[stylesForMotion.stage, {opacity, transform: [{translateY}]}]}>{children}</Animated.View>;
}

export const stylesForMotion = StyleSheet.create({
  stage: {flex: 1},
});
