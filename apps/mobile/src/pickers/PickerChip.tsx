import React from 'react';
import {type MobileModelOption} from '../modelPicker';
import {MilimIcon, type MilimIconName} from '../ui/MilimIcon';
import {ProviderIcon} from '../ui/ProviderIcon';
import {useAppTheme, Text} from '../ui/appTheme';
import {MotionPressable} from '../ui/motion';

export function PickerChip({
  label,
  icon,
  leading,
  providerBrand,
  onPress,
  warning,
}: {
  label: string;
  icon: MilimIconName;
  leading?: React.ReactNode;
  providerBrand?: MobileModelOption['brand'];
  onPress: () => void;
  warning?: boolean;
}) {
  const {palette, styles} = useAppTheme();
  return (
    <MotionPressable style={[styles.chip, warning && styles.warningChip]} hitSlop={8} onPress={onPress}>
      {leading ?? (providerBrand !== undefined
        ? <ProviderIcon brand={providerBrand} size={12} color={warning ? palette.warning : palette.secondary} />
        : <MilimIcon name={icon} size={12} color={warning ? palette.warning : palette.secondary} />)}
      <Text style={styles.chipText} numberOfLines={1}>{label}</Text>
      <MilimIcon name="chevron-down" size={12} color={palette.muted} />
    </MotionPressable>
  );
}
