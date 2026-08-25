import fs from 'node:fs';
import path from 'node:path';

const mobileRoot = path.resolve(__dirname, '..');
const repoRoot = path.resolve(mobileRoot, '../..');

function read(...parts: string[]): string {
  return fs.readFileSync(path.join(mobileRoot, ...parts), 'utf8');
}

function occurrences(value: string, needle: string): number {
  return value.split(needle).length - 1;
}

function pngDimensions(filePath: string): {width: number; height: number; colorType: number} {
  const image = fs.readFileSync(filePath);
  expect(image.subarray(0, 8)).toEqual(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]));
  return {
    width: image.readUInt32BE(16),
    height: image.readUInt32BE(20),
    colorType: image[25],
  };
}

test('mobile app identifiers and versions match the release-ready configuration', () => {
  const version = fs.readFileSync(path.join(repoRoot, 'VERSION'), 'utf8').trim();
  const packageJson = JSON.parse(read('package.json')) as {version: string};
  const android = read('android', 'app', 'build.gradle');
  const settings = read('android', 'settings.gradle');
  const activity = read(
    'android',
    'app',
    'src',
    'main',
    'java',
    'com',
    'omershatz',
    'milim',
    'MainActivity.kt',
  );
  const application = read(
    'android',
    'app',
    'src',
    'main',
    'java',
    'com',
    'omershatz',
    'milim',
    'MainApplication.kt',
  );
  const project = read('ios', 'MilimMobile.xcodeproj', 'project.pbxproj');

  expect(packageJson.version).toBe(version);
  expect(android).toContain('namespace "com.omershatz.milim"');
  expect(android).toContain('applicationId "com.omershatz.milim"');
  expect(android).toContain(`?: "${version}"`);
  expect(settings).toContain("rootProject.name = 'com.omershatz.milim'");
  expect(activity).toContain('package com.omershatz.milim');
  expect(application).toContain('package com.omershatz.milim');
  expect(occurrences(project, `MARKETING_VERSION = ${version};`)).toBe(2);
  expect(occurrences(project, 'CURRENT_PROJECT_VERSION = 1;')).toBe(2);
  expect(occurrences(project, 'PRODUCT_BUNDLE_IDENTIFIER = "com.omershatz.milim";')).toBe(2);
  expect(occurrences(project, 'TARGETED_DEVICE_FAMILY = 1;')).toBe(2);
  expect(project).not.toContain('TARGETED_DEVICE_FAMILY = "1,2";');
});

test('mobile store delivery preserves protected, retry-safe release invariants', () => {
  const workflow = fs.readFileSync(
    path.join(repoRoot, '.github', 'workflows', 'mobile-store.yml'),
    'utf8',
  );

  expect(workflow).toContain('environment: mobile-store-ios');
  expect(workflow).toContain('environment: mobile-store-android');
  expect(workflow).toMatch(/options:\s+- both\s+- ios\s+- android/);
  expect(occurrences(workflow, 'RELEASE_TAG: ${{ inputs.release_tag }}')).toBe(2);
  expect(workflow).not.toContain('expected="${{ inputs.release_tag }}"');
  expect(
    occurrences(workflow, 'git show-ref --verify --quiet "refs/tags/$RELEASE_TAG"'),
  ).toBe(2);
  expect(workflow).toContain('GITHUB_RUN_NUMBER * 100 + GITHUB_RUN_ATTEMPT');
  expect(workflow).not.toContain('ios_build_number:');
  expect(workflow).not.toContain('android_version_code:');
  expect(workflow).toContain('provisioningProfiles:com.omershatz.milim');
  expect(workflow).toContain('packageName: com.omershatz.milim');
  expect(workflow).not.toContain('com.milim.mobile');
  expect(workflow).toContain('CODE_SIGN_IDENTITY="Apple Distribution"');
  expect(workflow).toContain('codesign --verify --deep --strict');
  expect(workflow).toContain('jarsigner -verify');
  expect(workflow).toContain(
    'r0adkll/upload-google-play@e738b9dd8f2476ea806d921b64aacd24f34515a5',
  );
  expect(workflow).toContain('track: internal');
  expect(workflow).not.toMatch(/^\s+track:\s+production\s*$/m);
});

test('iOS metadata preserves pairing and bundles the privacy manifest', () => {
  const info = read('ios', 'MilimMobile', 'Info.plist');
  const privacy = read('ios', 'MilimMobile', 'PrivacyInfo.xcprivacy');
  const project = read('ios', 'MilimMobile.xcodeproj', 'project.pbxproj');
  const launchScreen = read('ios', 'MilimMobile', 'LaunchScreen.storyboard');

  expect(info).toContain('<string>com.omershatz.milim.pairing</string>');
  expect(info).toMatch(/<key>CFBundleURLSchemes<\/key>[\s\S]*<string>milim<\/string>/);
  expect(info).toMatch(/<key>ITSAppUsesNonExemptEncryption<\/key>\s*<false\/>/);
  expect(privacy).toContain('<string>C617.1</string>');
  expect(privacy).toContain('<string>CA92.1</string>');
  expect(privacy).toContain('<string>35F9.1</string>');
  expect(privacy).toMatch(/<key>NSPrivacyCollectedDataTypes<\/key>\s*<array\/>/);
  expect(privacy).toMatch(/<key>NSPrivacyTracking<\/key>\s*<false\/>/);
  expect(project).toContain('PrivacyInfo.xcprivacy in Resources');
  expect(project).toMatch(/PBXResourcesBuildPhase[\s\S]*13B07FC01A68108700A75B9A \/\* PrivacyInfo\.xcprivacy in Resources \*\//);
  expect(launchScreen).toContain('image="MilimLaunchMark"');
  expect(launchScreen).not.toContain('text="milim"');
  expect(launchScreen).toContain('constant="72"');
  expect(launchScreen).toContain('systemColor="systemBackgroundColor"');
  expect(launchScreen).not.toContain('Powered by React Native');
});

test('iOS launch screen bundles a smaller transparent milim mark', () => {
  const launchAssetRoot = path.join(
    mobileRoot,
    'ios',
    'MilimMobile',
    'Images.xcassets',
    'MilimLaunchMark.imageset',
  );
  const catalog = read(
    'ios',
    'MilimMobile',
    'Images.xcassets',
    'MilimLaunchMark.imageset',
    'Contents.json',
  );
  const expected = new Map<string, number>([
    ['milim-launch-mark.png', 72],
    ['milim-launch-mark@2x.png', 144],
    ['milim-launch-mark@3x.png', 216],
  ]);

  expect(catalog).toContain('"template-rendering-intent" : "template"');
  for (const [filename, size] of expected) {
    expect(pngDimensions(path.join(launchAssetRoot, filename))).toEqual({
      width: size,
      height: size,
      colorType: 6,
    });
  }
});

test('iOS picker and drawer source keep one reasoning sparkle and modal-safe insets', () => {
  const app = read('App.tsx');
  expect(app).toContain("capability !== 'reasoning'");
  expect(app).toContain("pickerRowMeta: {flexDirection: 'row', alignItems: 'center', gap: 6, minHeight: 14}");
  expect(app).toContain("capability === 'fast' && styles.pickerCapabilityFastIcon");
  expect(app).toContain('pickerCapabilityFastIcon: {transform: [{translateY: -1}]}');
  expect(app).toMatch(/<Modal visible=\{visible\}[\s\S]*<SafeAreaProvider>[\s\S]*<SafeAreaView style=\{styles\.drawerSafe\} edges=\{\['top', 'left', 'bottom'\]\}>/);
  expect(app).toContain('styles.drawerEdgeFade, {opacity: edgeOpacity}');
});

test('mobile threads follow the native keyboard without losing the latest message', () => {
  const app = read('App.tsx');
  expect(app).toContain("Keyboard.addListener('keyboardWillShow', show)");
  expect(app).toContain("Keyboard.addListener('keyboardWillHide', hide)");
  expect(app).toContain('duration: 120');
  expect(app).toContain('endCoordinates.height - safeBottom');
  expect(app).toContain('behavior="height"');
  expect(app).toContain("Keyboard.addListener('keyboardDidShow', keepLatestVisible)");
  expect(app).toContain("Keyboard.addListener('keyboardDidHide', keepLatestVisible)");
  expect(app).toContain('if (!followingLatest.current) return;');
});

test('mobile chat chrome stays quiet and compact', () => {
  const app = read('App.tsx');
  const avatar = read('src', 'ui', 'AgentAvatar.tsx');

  expect(app).not.toContain('styles.threadStatus');
  expect(app).not.toContain("thread.model || 'No model'");
  expect(app).toContain('threadCard: {minHeight: 46');
  expect(app).toContain('reasoningBlock: {paddingBottom: 7, marginBottom: 6}');
  expect(avatar).toContain("texture: 'grain'");
  expect(avatar).toContain('overlayGradient: true');
  expect(avatar).toContain('dither: false');
});

test('iOS app icon catalog contains complete opaque assets', () => {
  const iconRoot = path.join(
    mobileRoot,
    'ios',
    'MilimMobile',
    'Images.xcassets',
    'AppIcon.appiconset',
  );
  const catalog = JSON.parse(
    fs.readFileSync(path.join(iconRoot, 'Contents.json'), 'utf8'),
  ) as {images: Array<{filename?: string}>};
  const expected = new Map<string, number>([
    ['AppIcon-20@2x.png', 40],
    ['AppIcon-20@3x.png', 60],
    ['AppIcon-29@2x.png', 58],
    ['AppIcon-29@3x.png', 87],
    ['AppIcon-40@2x.png', 80],
    ['AppIcon-40@3x.png', 120],
    ['AppIcon-60@2x.png', 120],
    ['AppIcon-60@3x.png', 180],
    ['AppIcon-1024.png', 1024],
  ]);

  expect(catalog.images.map(image => image.filename)).toEqual([...expected.keys()]);
  for (const [filename, size] of expected) {
    expect(pngDimensions(path.join(iconRoot, filename))).toEqual({
      width: size,
      height: size,
      colorType: 2,
    });
  }
});
