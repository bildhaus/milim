import {
  createMobileTheme,
  defaultAppearance,
  mobileBackgroundResizeMode,
} from '../src/theme';

test('maps a host appearance snapshot into native mobile tokens', () => {
  const theme = createMobileTheme({
    ...defaultAppearance,
    revision: 'custom-1',
    theme_id: 'violet-host',
    is_dark: false,
    colors: {
      ...defaultAppearance.colors,
      bg_primary: '#f4f1ff',
      card_bg: '#ffffff',
      accent: '#6d28d9',
      primary_text: '#25133d',
    },
    borders: {...defaultAppearance.borders, card_radius: 20, input_radius: 16},
  });

  expect(theme.palette.bg).toBe('#f4f1ff');
  expect(theme.palette.accent).toBe('#6d28d9');
  expect(theme.palette.accentInk).toBe('#ffffff');
  expect(theme.cardRadius).toBe(20);
  expect(theme.inputRadius).toBe(16);
  expect(theme.isDark).toBe(false);
});

test('falls back safely when no desktop appearance is available', () => {
  const theme = createMobileTheme();
  expect(theme.appearance.theme_id).toBe('mono-dark');
  expect(theme.palette.bg).toBe('#0d0d0f');
});

test('maps desktop background fits to aspect-preserving native modes', () => {
  expect(mobileBackgroundResizeMode('cover')).toBe('cover');
  expect(mobileBackgroundResizeMode('fill')).toBe('cover');
  expect(mobileBackgroundResizeMode('contain')).toBe('contain');
  expect(mobileBackgroundResizeMode('center')).toBe('center');
  expect(mobileBackgroundResizeMode('tile')).toBe('repeat');
  expect(mobileBackgroundResizeMode('unknown')).toBe('cover');
});
