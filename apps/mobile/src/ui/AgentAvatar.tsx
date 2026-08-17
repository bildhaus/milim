import React, {useMemo} from 'react';
import {View, type StyleProp, type ViewStyle} from 'react-native';
import {SvgXml} from 'react-native-svg';
import {createAvatarRecipe, createAvatarSvg} from '@oshtz/shatz-avatars';
import {
  agentAvatarPalette,
  agentAvatarSeed,
  nativeAgentAvatarSvg,
  type AgentAvatarIdentity,
} from '../agentAvatar';

export function AgentAvatar({
  id,
  name,
  avatar,
  size = 24,
  style,
}: AgentAvatarIdentity & {
  size?: number;
  style?: StyleProp<ViewStyle>;
}) {
  const seed = agentAvatarSeed({id, name, avatar});
  const xml = useMemo(() => {
    const palette = agentAvatarPalette(createAvatarRecipe(seed).shape);
    return nativeAgentAvatarSvg(createAvatarSvg({
      seed,
      shape: 'circle',
      texture: 'solid',
      blur: 0,
      noise: 0,
      dither: false,
      overlayGradient: false,
      color: palette.color,
      secondaryColor: palette.secondaryColor,
      background: palette.background,
      size: 100,
      title: '',
    }));
  }, [seed]);

  return (
    <View
      pointerEvents="none"
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={[{width: size, height: size}, style]}>
      <SvgXml xml={xml} width={size} height={size} />
    </View>
  );
}
