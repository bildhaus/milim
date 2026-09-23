import {trigger} from 'react-native-haptic-feedback';

// Feedback is a nicety: a device without a haptic engine, or a user who turned
// system haptics off, must never turn a tap into an error.
const OPTIONS = {enableVibrateFallback: false, ignoreAndroidSystemSettings: false} as const;

function play(type: 'selection' | 'impactLight' | 'notificationSuccess' | 'notificationWarning') {
  try {
    trigger(type, OPTIONS);
  } catch {
    // Ignored on purpose; see above.
  }
}

export const haptics = {
  // A picker row, drawer commit, or long-press menu.
  selection: () => play('selection'),
  // Sending or steering a turn.
  send: () => play('impactLight'),
  // An approval was granted, or text was copied.
  success: () => play('notificationSuccess'),
  // An approval was denied or a destructive action confirmed.
  warning: () => play('notificationWarning'),
};
