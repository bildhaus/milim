# milim mobile

This is milim's native iOS and Android companion. It is a bare React Native + TypeScript + Metro project generated with the React Native community CLI. It deliberately contains no Expo or EAS packages.

The iOS target currently supports iPhone only. Android remains supported through the native Gradle project. The iOS Home Screen icon stays opaque with a slightly inset mark; the launch screen shows a smaller transparent milim mark instead of the app name.

The application is a direct, multi-host controller for a running milim desktop. It keeps host-partitioned bounded timeline caches and drafts in mobile SQLite, stores device credentials in Keychain/Keystore, and connects over Tailscale, opt-in LAN/mDNS, or a manual URL. Both first-run pairing and Pair another desktop scan for nearby advertised desktops, identify them by their OS computer names, and hide desktops already saved on the phone. Tapping a discovered desktop creates a private, expiring request that must be approved on desktop before mobile can claim a device credential. The phone shows request, waiting, rejection, cancellation, expiry, and connecting states, while QR/deep-link pairing remains a collapsed fallback for remote or discovery-blocked networks. When a saved Tailscale endpoint is unreachable, the phone explains that Tailscale should be active on both devices and offers an immediate connection retry without changing the saved pairing. Development builds also probe the desktop's preferred LAN port through `10.0.2.2` in the Android emulator and `127.0.0.1` in the iOS simulator; release builds do not perform these simulator probes. Live events are flushed once per display frame and projected incrementally, preserving unchanged transcript-row identities; only sequence gaps, epoch changes, prepends, and replacement events rebuild authoritative state. Drafts and bounded timeline snapshots update immediately in memory but persist after short coalescing windows, with drafts flushed when sending, changing hosts or threads, backgrounding, or unmounting. Its local transcript projection interleaves messages with grouped work, tool and file-change rows, failures, and inline approvals while Attention remains the aggregate inbox. The active host's interface and code font stacks map to native platform families or their serif, sans-serif, and monospace equivalents. The mobile shell has no persistent bottom navigation: the leading header button or a rightward swipe from the left edge opens the left-side thread drawer, the host identity opens Hosts, the connection-status chip opens Attention and carries its pending count, and each secondary screen has a direct return to Chat. The drawer closes from its button, backdrop, platform Back action, or a leftward swipe. The composer stays expanded at the latest turn or while active and collapses to a translucent pill only while reading earlier content; expanding it preserves the transcript position. Choosing Latest holds that compact state through the return, then expands once while the transcript remains pinned above its final height. Sending goes directly to the bottom. It does not auto-send offline drafts or decisions and does not promise background connectivity or push notifications.

The model control uses the desktop provider/runtime brand set in the composer, group headers, and model rows. Its searchable sheet reads and mutates the desktop-host's canonical favorite model IDs, so favorite changes propagate live between desktop and paired phones. Favorites-only filtering and collapsed groups remain host-partitioned presentation preferences in mobile SQLite. The sheet renders route/context and capability metadata compactly and writes supported reasoning-effort choices back to the canonical thread. When paired to an older desktop without canonical favorite support, the phone retains its previous host-local favorites behavior. The Agent control renders the same deterministic `@oshtz/shatz-avatars` identity in the composer and searchable Agent sheet, using a native-safe SVG pass for iOS and Android. An upstream `owned_by: "milim"` placeholder is displayed as **Local models** with a publisher namespace when the model ID provides one. Provider connection tests, refreshes, saves, and deletions publish a live catalog event so a connected phone reloads LM Studio and other provider models without reconnecting.

From the repository root:

```sh
pnpm -C apps/mobile install --frozen-lockfile
pnpm -C apps/mobile verify
pnpm -C apps/mobile android
```

Binary attachments remain as local URIs in JavaScript. When the paired desktop advertises `attachment_uploads`, the phone streams each file through the native blob transport to the authenticated control upload route and sends only its temporary `upload_id`; the desktop resolves that upload into the canonical attachment during command acceptance. Uploads are limited to 2 MiB each and 12 pending per paired device, expire after 15 minutes, and repeated client attachment IDs are idempotent. Older desktops remain compatible: the phone performs the previous base64 conversion only immediately before sending. Text attachments keep the existing bounded 128 KiB prompt path.

## Mobile performance proof

Profile release/profile builds on one physical iPhone and one mid-range physical Android device. Record three runs of each scenario and compare medians: a 150-item cached tail, 1,000 loaded items, 20 streamed events per second for 30 seconds, typing while streaming, rapid thread switching, and six 2 MiB binary attachments. Use React Native DevTools with Xcode Instruments on iOS and Android Studio/Perfetto on Android. The app emits standard Performance API entries prefixed with `milim.mobile` for startup, event-batch flushes, transcript projection, draft persistence, and attachment upload; React Native also exposes `rnStartupTiming`.

The acceptance bar is at most one timeline React commit per display frame, at least 55 FPS on 60 Hz devices, no JavaScript long task over 100 ms, p95 input/event latency below 50 ms, and at least 30% less transcript JavaScript/React work than the pre-change baseline. Startup and non-attachment memory may not regress by more than 5%. Keep the existing fade, mask, and composer motion unless missed-frame traces attribute more than 10% of a frame to them in at least two of three runs. Metro `inlineRequires` is intentionally disabled until the same three-run cold-start measurement proves at least a 5% improvement without cycles or functional regressions.

For iOS on macOS, install CocoaPods from this directory, open `ios/MilimMobile.xcworkspace`, and run the `MilimMobile` scheme. Pair with Settings -> Mobile in the desktop app. The desktop process is authoritative and must remain running.

The remainder of this file is the upstream bare React Native development reference.

---

This project was bootstrapped using [`@react-native-community/cli`](https://github.com/react-native-community/cli).

# Getting Started

> **Note**: Make sure you have completed the [Set Up Your Environment](https://reactnative.dev/docs/set-up-your-environment) guide before proceeding.

## Step 1: Start Metro

First, you will need to run **Metro**, the JavaScript build tool for React Native.

To start the Metro dev server, run the following command from the root of your React Native project:

```sh
# Using npm
npm start

# OR using Yarn
yarn start
```

## Step 2: Build and run your app

With Metro running, open a new terminal window/pane from the root of your React Native project, and use one of the following commands to build and run your Android or iOS app:

### Android

```sh
# Using npm
npm run android

# OR using Yarn
yarn android
```

### iOS

For iOS, remember to install CocoaPods dependencies (this only needs to be run on first clone or after updating native deps).

The first time you create a new project, run the Ruby bundler to install CocoaPods itself:

```sh
bundle install
```

Then, and every time you update your native dependencies, run:

```sh
bundle exec pod install
```

For more information, please visit [CocoaPods Getting Started guide](https://guides.cocoapods.org/using/getting-started.html).

```sh
# Using npm
npm run ios

# OR using Yarn
yarn ios
```

If everything is set up correctly, you should see your new app running in the Android Emulator, iOS Simulator, or your connected device.

This is one way to run your app — you can also build it directly from Android Studio or Xcode.

## Step 3: Modify your app

Now that you have successfully run the app, let's make changes!

Open `App.tsx` in your text editor of choice and make some changes. When you save, your app will automatically update and reflect these changes — this is powered by [Fast Refresh](https://reactnative.dev/docs/fast-refresh).

When you want to forcefully reload, for example to reset the state of your app, you can perform a full reload:

- **Android**: Press the <kbd>R</kbd> key twice or select **"Reload"** from the **Dev Menu**, accessed via <kbd>Ctrl</kbd> + <kbd>M</kbd> (Windows/Linux) or <kbd>Cmd ⌘</kbd> + <kbd>M</kbd> (macOS).
- **iOS**: Press <kbd>R</kbd> in iOS Simulator.

## Congratulations! :tada:

You've successfully run and modified your React Native App. :partying_face:

### Now what?

- If you want to add this new React Native code to an existing application, check out the [Integration guide](https://reactnative.dev/docs/integration-with-existing-apps).
- If you're curious to learn more about React Native, check out the [docs](https://reactnative.dev/docs/getting-started).

# Troubleshooting

If you're having issues getting the above steps to work, see the [Troubleshooting](https://reactnative.dev/docs/troubleshooting) page.

# Learn More

To learn more about React Native, take a look at the following resources:

- [React Native Website](https://reactnative.dev) - learn more about React Native.
- [Getting Started](https://reactnative.dev/docs/environment-setup) - an **overview** of React Native and how setup your environment.
- [Learn the Basics](https://reactnative.dev/docs/getting-started) - a **guided tour** of the React Native **basics**.
- [Blog](https://reactnative.dev/blog) - read the latest official React Native **Blog** posts.
- [`@facebook/react-native`](https://github.com/facebook/react-native) - the Open Source; GitHub **repository** for React Native.
