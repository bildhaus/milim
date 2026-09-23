import {View} from 'react-native';
import {MilimIcon, type MilimIconName} from './MilimIcon';
import {useAppTheme, Text} from './appTheme';
import {MotionPressable} from './motion';

export function Button({
  label,
  icon,
  onPress,
  tone = 'accent',
  disabled,
}: {
  label: string;
  icon?: MilimIconName;
  onPress: () => void;
  tone?: 'accent' | 'quiet' | 'danger';
  disabled?: boolean;
}) {
  const {palette, styles} = useAppTheme();
  const iconColor = tone === 'accent'
    ? palette.accentInk
    : tone === 'danger'
      ? palette.danger
      : palette.text;
  return (
    <MotionPressable style={[styles.button, tone === 'quiet' && styles.buttonQuiet, tone === 'danger' && styles.buttonDanger, disabled && styles.disabled]} onPress={onPress} disabled={disabled}>
      {icon ? <MilimIcon name={icon} size={15} color={iconColor} /> : null}
      <Text style={[styles.buttonText, tone === 'quiet' && styles.buttonQuietText, tone === 'danger' && styles.buttonDangerText]}>{label}</Text>
    </MotionPressable>
  );
}

export function IconButton({
  icon,
  label,
  onPress,
  tone = 'quiet',
  disabled,
}: {
  icon: MilimIconName;
  label: string;
  onPress: () => void;
  tone?: 'quiet' | 'accent';
  disabled?: boolean;
}) {
  const {palette, styles} = useAppTheme();
  return (
    <MotionPressable
      style={[styles.iconButton, tone === 'accent' && styles.iconButtonAccent, disabled && styles.disabled]}
      accessibilityLabel={label}
      onPress={onPress}
      disabled={disabled}
      hitSlop={4}>
      <MilimIcon name={icon} size={tone === 'accent' ? 17 : 15} color={tone === 'accent' ? palette.accentInk : palette.secondary} />
    </MotionPressable>
  );
}

export function ActionSheetButton({icon, label, onPress, danger}: {icon: MilimIconName; label: string; onPress: () => void; danger?: boolean}) {
  const {palette, styles} = useAppTheme();
  const color = danger ? palette.danger : palette.secondary;
  return (
    <MotionPressable style={styles.actionSheetButton} onPress={onPress}>
      <View style={styles.actionSheetIcon}>
        <MilimIcon name={icon} size={16} color={color} />
      </View>
      <Text style={[styles.actionSheetLabel, danger && styles.smallDanger]}>{label}</Text>
    </MotionPressable>
  );
}

export function Empty({title, copy, action, onAction}: {title: string; copy: string; action?: string; onAction?: () => void}) {
  const {styles} = useAppTheme();
  return (
    <View style={styles.empty}>
      <Text style={styles.emptyTitle} accessibilityRole="header">{title}</Text>
      <Text style={[styles.help, styles.emptyCopy]}>{copy}</Text>
      {action && onAction ? <Button label={action} tone="quiet" onPress={onAction} /> : null}
    </View>
  );
}
