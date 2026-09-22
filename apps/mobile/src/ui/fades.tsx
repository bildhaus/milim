import {Animated, StyleSheet} from 'react-native';
import Svg, {Defs, LinearGradient as SvgLinearGradient, Rect, Stop} from 'react-native-svg';
import {TRANSCRIPT_FADE_HEIGHT} from './constants';

export function SheetBackdropFade() {
  return (
    <Svg pointerEvents="none" style={StyleSheet.absoluteFill} width="100%" height="100%">
      <Defs>
        <SvgLinearGradient id="sheet-backdrop-fade" x1="0" y1="0" x2="0" y2="1">
          <Stop offset="0" stopColor="#000" stopOpacity="0" />
          <Stop offset="0.14" stopColor="#000" stopOpacity="0.24" />
          <Stop offset="0.28" stopColor="#000" stopOpacity="0.48" />
          <Stop offset="1" stopColor="#000" stopOpacity="0.48" />
        </SvgLinearGradient>
      </Defs>
      <Rect width="100%" height="100%" fill="url(#sheet-backdrop-fade)" />
    </Svg>
  );
}

export function DrawerBackdropFade() {
  return (
    <Svg pointerEvents="none" style={StyleSheet.absoluteFill} width="100%" height="100%">
      <Defs>
        <SvgLinearGradient id="drawer-backdrop-fade" x1="0" y1="0" x2="1" y2="0">
          <Stop offset="0" stopColor="#000" stopOpacity="0.54" />
          <Stop offset="0.3" stopColor="#000" stopOpacity="0.42" />
          <Stop offset="0.65" stopColor="#000" stopOpacity="0.18" />
          <Stop offset="1" stopColor="#000" stopOpacity="0" />
        </SvgLinearGradient>
      </Defs>
      <Rect width="100%" height="100%" fill="url(#drawer-backdrop-fade)" />
    </Svg>
  );
}

export function TranscriptFadeOverlay({
  bottomInset,
  color,
}: {
  bottomInset: Animated.AnimatedInterpolation<number>;
  color: string;
}) {
  return (
    <Animated.View
      pointerEvents="none"
      style={[stylesStatic.transcriptFadeOverlay, {bottom: bottomInset}]}>
      <Svg width="100%" height="100%">
        <Defs>
          <SvgLinearGradient id="transcript-fade-overlay" x1="0" y1="0" x2="0" y2="1">
            <Stop offset="0" stopColor={color} stopOpacity="0" />
            <Stop offset="1" stopColor={color} stopOpacity="0.96" />
          </SvgLinearGradient>
        </Defs>
        <Rect width="100%" height="100%" fill="url(#transcript-fade-overlay)" />
      </Svg>
    </Animated.View>
  );
}

export const stylesStatic = StyleSheet.create({
  transcriptFadeOverlay: {
    position: 'absolute',
    right: 0,
    left: 0,
    zIndex: 1,
    height: TRANSCRIPT_FADE_HEIGHT,
  },
});
