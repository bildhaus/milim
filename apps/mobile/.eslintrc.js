module.exports = {
  root: true,
  extends: '@react-native',
  rules: {
    // UI event handlers intentionally mark detached promises with `void`.
    'no-void': 'off',
  },
};
