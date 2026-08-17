import fs from 'node:fs';
import path from 'node:path';

test('Android packages the animated GIF decoder matching React Native Fresco', () => {
  const buildGradle = fs.readFileSync(
    path.resolve(__dirname, '../android/app/build.gradle'),
    'utf8',
  );
  const reactNativeVersions = fs.readFileSync(
    path.resolve(__dirname, '../node_modules/react-native/gradle/libs.versions.toml'),
    'utf8',
  );
  const frescoVersion = reactNativeVersions.match(/^fresco = "([^"]+)"$/m)?.[1];

  expect(frescoVersion).toBeDefined();
  expect(buildGradle).toContain(
    `implementation("com.facebook.fresco:animated-gif:${frescoVersion}")`,
  );
});
