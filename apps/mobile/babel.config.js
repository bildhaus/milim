module.exports = {
  presets: ['module:@react-native/babel-preset'],
  // Must stay last: it compiles Reanimated and gesture worklets.
  plugins: ['react-native-worklets/plugin'],
};
