import {createContext, useContext} from 'react';
import {Platform, Text as NativeText, type TextProps} from 'react-native';
import {type AppearanceSnapshotV1} from '../control/types';
import {createMobileTheme} from '../theme';
import {createMarkdownStyles, createStyles} from './styles';

export function appTheme(snapshot?: AppearanceSnapshotV1) {
  const theme = createMobileTheme(snapshot, Platform.OS === 'ios' ? 'ios' : 'android');
  return {
    ...theme,
    styles: createStyles(theme),
    markdownStyles: createMarkdownStyles(theme),
  };
}

export type AppTheme = ReturnType<typeof appTheme>;
export const AppThemeContext = createContext<AppTheme>(appTheme());

export function useAppTheme(): AppTheme {
  return useContext(AppThemeContext);
}

export function Text({style, ...props}: TextProps) {
  const {styles} = useAppTheme();
  return <NativeText {...props} style={[styles.appText, style]} />;
}
