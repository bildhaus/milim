import {Image, StyleSheet, View} from 'react-native';
import {mobileBackgroundResizeMode} from '../theme';
import {useAppTheme} from './appTheme';

export function AppearanceBackground({uri}: {uri: string | null}) {
  const {appearance, palette, styles} = useAppTheme();
  const background = appearance.background;
  const treatment = background.treatment ?? 'clear';
  const resizeMode = mobileBackgroundResizeMode(background.fit);
  const opacity = Math.max(0, Math.min(1, background.image_opacity ?? 1));
  const overlayOpacity = Math.max(0, Math.min(1, background.overlay_opacity ?? 0));
  const blurRadius = Math.max(0, background.image_blur ?? 0) + (treatment === 'blur' ? 8 : 0);
  return (
    <View style={[StyleSheet.absoluteFill, {backgroundColor: palette.bg}]} pointerEvents="none">
      {uri ? (
        <View
          style={[
            StyleSheet.absoluteFill,
            treatment === 'mono' ? {filter: [{grayscale: 1}]} : null,
          ]}>
          <Image
            source={{uri}}
            style={[
              styles.backgroundImage,
              resizeMode === 'cover' && styles.backgroundImageCover,
              {opacity},
            ]}
            resizeMode={resizeMode}
            blurRadius={blurRadius}
          />
        </View>
      ) : null}
      {overlayOpacity > 0 ? (
        <View style={[StyleSheet.absoluteFill, {backgroundColor: background.overlay_color ?? '#000000', opacity: overlayOpacity}]} />
      ) : null}
      {treatment === 'dim' ? <View style={[StyleSheet.absoluteFill, styles.backgroundDim]} /> : null}
    </View>
  );
}
