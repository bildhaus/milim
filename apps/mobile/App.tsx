import React, {createContext, useCallback, useContext, useEffect, useMemo, useRef, useState} from 'react';
import {
  AccessibilityInfo,
  ActivityIndicator,
  Alert,
  Animated,
  FlatList,
  Image,
  Keyboard,
  KeyboardAvoidingView,
  LayoutAnimation,
  Linking,
  Modal,
  PanResponder,
  PermissionsAndroid,
  Platform,
  Pressable,
  ScrollView,
  SectionList,
  StatusBar,
  StyleSheet,
  Text as NativeText,
  TextInput,
  View,
  useWindowDimensions,
  type StyleProp,
  type KeyboardEvent,
  type TextProps,
  type ViewStyle,
} from 'react-native';
import {SafeAreaProvider, SafeAreaView, useSafeAreaInsets} from 'react-native-safe-area-context';
import Markdown, {MarkdownIt, type RenderRules} from 'react-native-markdown-display';
import Svg, {Defs, LinearGradient as SvgLinearGradient, Rect, Stop} from 'react-native-svg';
import {
  cleanupAttachments,
  cleanupStaleAttachments,
  MAX_ATTACHMENTS,
  pickFiles,
  pickPhoto,
  promptWithAttachments,
} from './src/attachments';
import {MOBILE_MARKDOWN_OPTIONS} from './src/markdown';
import {newCommandId} from './src/control/client';
import type {RunEventPageV1, RunEventV1, RunInspectionV1} from './src/control/generated-v1';
import {
  projectTranscriptIncrementally,
  type ActivityStatus,
  type ProjectedActivityGroup,
  type ProjectedActivityIcon,
  type ProjectedActivityRow,
  type ProjectedTranscriptItem,
  type TranscriptProjectionCache,
} from './src/control/replica';
import type {
  AppearanceSnapshotV1,
  ControlAttachmentV1,
  ControlCommandV1,
  JsonValue,
  PendingApprovalV1,
  QueuedTurnV1,
  ThreadSummaryV1,
} from './src/control/types';
import {
  useMilimController,
  type NearbyPairingStage,
} from './src/controller/useMilimController';
import {
  discoverMilimHosts,
  filterPairableMilimHosts,
  type DiscoveredHost,
} from './src/discovery';
import {
  DEFAULT_MODEL_PICKER_PREFERENCES,
  modelPickerFavoriteIds,
  modelPickerGroups,
  mobileModelOptions,
  transcriptModelLabel,
  toggledModelFavoriteIds,
  type MobileModelCapability,
  type MobileModelOption,
} from './src/modelPicker';
import {
  readModelPickerPreferences,
  saveModelPickerPreferences,
} from './src/storage/cache';
import {
  createMobileTheme,
  mobileBackgroundResizeMode,
  type MobilePalette,
  type MobileTheme,
} from './src/theme';
import {mobilePerfMark, mobilePerfMeasure, mobileStartupTiming} from './src/performance';
import {
  canUseCompactComposer,
  friendlyEndpoint,
  friendlyPairingError,
  groupMobileThreads,
  lowercaseMilimBrand,
  nextAwayFromLatest,
  relativeConnectionTime,
  shouldHoldCompactComposerForLatestReturn,
  transcriptDistanceFromLatest,
  type MobileThreadGroup,
} from './src/mobileUi';
import {MilimIcon, type MilimIconName} from './src/ui/MilimIcon';
import {AgentAvatar} from './src/ui/AgentAvatar';
import {ProviderIcon} from './src/ui/ProviderIcon';

type AppScreen = 'chat' | 'attention' | 'hosts';

const milimLogo = require('./src/assets/milim-icon.png');
const TRANSCRIPT_FADE_HEIGHT = 40;
const DRAWER_EDGE_GESTURE_WIDTH = 28;
const DRAWER_SWIPE_ACTIVATION_DISTANCE = 8;
const DRAWER_SWIPE_COMMIT_DISTANCE = 52;
const DRAWER_SWIPE_COMMIT_VELOCITY = 0.5;

function markdownCodeContent(content: string): string {
  return content.endsWith('\n') ? content.slice(0, -1) : content;
}

const mobileMarkdownRules: RenderRules = {
  code_block: (node, _children, _parents, styles, inheritedStyles = {}) => (
    <ScrollView key={node.key} horizontal nestedScrollEnabled showsHorizontalScrollIndicator>
      <Text style={[inheritedStyles, styles.code_block]} selectable>
        {markdownCodeContent(node.content)}
      </Text>
    </ScrollView>
  ),
  fence: (node, _children, _parents, styles, inheritedStyles = {}) => (
    <ScrollView key={node.key} horizontal nestedScrollEnabled showsHorizontalScrollIndicator>
      <Text style={[inheritedStyles, styles.fence]} selectable>
        {markdownCodeContent(node.content)}
      </Text>
    </ScrollView>
  ),
  table: (node, children, _parents, styles) => (
    <ScrollView key={node.key} horizontal nestedScrollEnabled showsHorizontalScrollIndicator>
      <View style={styles._VIEW_SAFE_table}>{children}</View>
    </ScrollView>
  ),
};

const mobileMarkdownParser = new MarkdownIt(MOBILE_MARKDOWN_OPTIONS);

function appTheme(snapshot?: AppearanceSnapshotV1) {
  const theme = createMobileTheme(snapshot, Platform.OS === 'ios' ? 'ios' : 'android');
  return {
    ...theme,
    styles: createStyles(theme),
    markdownStyles: createMarkdownStyles(theme),
  };
}

type AppTheme = ReturnType<typeof appTheme>;
const AppThemeContext = createContext<AppTheme>(appTheme());

function useAppTheme(): AppTheme {
  return useContext(AppThemeContext);
}

function Text({style, ...props}: TextProps) {
  const {styles} = useAppTheme();
  return <NativeText {...props} style={[styles.appText, style]} />;
}

function useReducedMotion(): boolean {
  const [reduced, setReduced] = useState(false);
  useEffect(() => {
    void AccessibilityInfo.isReduceMotionEnabled().then(setReduced);
    const subscription = AccessibilityInfo.addEventListener('reduceMotionChanged', setReduced);
    return () => subscription.remove();
  }, []);
  return reduced;
}

function SheetBackdropFade() {
  return (
    <Svg pointerEvents="none" style={StyleSheet.absoluteFill} width="100%" height="100%">
      <Defs>
        <SvgLinearGradient id="sheet-backdrop-fade" x1="0" y1="0" x2="0" y2="1">
          <Stop offset="0" stopColor="#000" stopOpacity="0" />
          <Stop offset="0.14" stopColor="#000" stopOpacity="0.24" />
          <Stop offset="0.28" stopColor="#000" stopOpacity="0.48" />
          <Stop offset="1" stopColor="#000" stopOpacity="0.48" />
        </SvgLinearGradient>
      </Defs>
      <Rect width="100%" height="100%" fill="url(#sheet-backdrop-fade)" />
    </Svg>
  );
}

function DrawerBackdropFade() {
  return (
    <Svg pointerEvents="none" style={StyleSheet.absoluteFill} width="100%" height="100%">
      <Defs>
        <SvgLinearGradient id="drawer-backdrop-fade" x1="0" y1="0" x2="1" y2="0">
          <Stop offset="0" stopColor="#000" stopOpacity="0.54" />
          <Stop offset="0.3" stopColor="#000" stopOpacity="0.42" />
          <Stop offset="0.65" stopColor="#000" stopOpacity="0.18" />
          <Stop offset="1" stopColor="#000" stopOpacity="0" />
        </SvgLinearGradient>
      </Defs>
      <Rect width="100%" height="100%" fill="url(#drawer-backdrop-fade)" />
    </Svg>
  );
}

function TranscriptFadeOverlay({
  bottomInset,
  color,
}: {
  bottomInset: Animated.AnimatedInterpolation<number>;
  color: string;
}) {
  return (
    <Animated.View
      pointerEvents="none"
      style={[stylesStatic.transcriptFadeOverlay, {bottom: bottomInset}]}>
      <Svg width="100%" height="100%">
        <Defs>
          <SvgLinearGradient id="transcript-fade-overlay" x1="0" y1="0" x2="0" y2="1">
            <Stop offset="0" stopColor={color} stopOpacity="0" />
            <Stop offset="1" stopColor={color} stopOpacity="0.96" />
          </SvgLinearGradient>
        </Defs>
        <Rect width="100%" height="100%" fill="url(#transcript-fade-overlay)" />
      </Svg>
    </Animated.View>
  );
}

const stylesStatic = StyleSheet.create({
  transcriptFadeOverlay: {
    position: 'absolute',
    right: 0,
    left: 0,
    zIndex: 1,
    height: TRANSCRIPT_FADE_HEIGHT,
  },
});

const AnimatedPressable = Animated.createAnimatedComponent(Pressable);

function MotionPressable({
  children,
  style,
  onPress,
  disabled,
  hitSlop,
  accessibilityLabel,
  accessibilityRole = 'button',
  accessibilityState,
}: {
  children: React.ReactNode;
  style?: StyleProp<ViewStyle>;
  onPress: () => void;
  disabled?: boolean;
  hitSlop?: number;
  accessibilityLabel?: string;
  accessibilityRole?: React.ComponentProps<typeof Pressable>['accessibilityRole'];
  accessibilityState?: React.ComponentProps<typeof Pressable>['accessibilityState'];
}) {
  const reduced = useReducedMotion();
  const scale = useRef(new Animated.Value(1)).current;
  const animate = (value: number, duration: number) => {
    if (reduced) {
      scale.setValue(value);
      return;
    }
    Animated.timing(scale, {
      toValue: value,
      duration,
      useNativeDriver: true,
    }).start();
  };
  return (
    <AnimatedPressable
      style={[style, {transform: [{scale}]}]}
      onPress={onPress}
      disabled={disabled}
      hitSlop={hitSlop}
      accessibilityRole={accessibilityRole}
      accessibilityLabel={accessibilityLabel}
      accessibilityState={accessibilityState}
      onPressIn={() => animate(0.97, 100)}
      onPressOut={() => animate(1, 140)}>
      {children}
    </AnimatedPressable>
  );
}

function ScreenStage({children}: {children: React.ReactNode}) {
  const reduced = useReducedMotion();
  const opacity = useRef(new Animated.Value(reduced ? 1 : 0)).current;
  const translateY = useRef(new Animated.Value(reduced ? 0 : 5)).current;
  useEffect(() => {
    if (reduced) {
      opacity.setValue(1);
      translateY.setValue(0);
      return;
    }
    Animated.parallel([
      Animated.timing(opacity, {toValue: 1, duration: 180, useNativeDriver: true}),
      Animated.timing(translateY, {toValue: 0, duration: 180, useNativeDriver: true}),
    ]).start();
  }, [opacity, reduced, translateY]);
  return <Animated.View style={[stylesForMotion.stage, {opacity, transform: [{translateY}]}]}>{children}</Animated.View>;
}

function ThreadKeyboardAvoidingView({
  children,
  enabled,
}: {
  children: React.ReactNode;
  enabled: boolean;
}) {
  const {styles} = useAppTheme();
  const reduced = useReducedMotion();
  const {bottom: safeBottom} = useSafeAreaInsets();
  const [keyboardInset, setKeyboardInset] = useState(0);

  const applyKeyboardInset = useCallback((nextInset: number) => {
    if (!reduced) {
      LayoutAnimation.configureNext({
        duration: 120,
        update: {duration: 120, type: LayoutAnimation.Types.easeOut},
      });
    }
    setKeyboardInset(nextInset);
  }, [reduced]);

  useEffect(() => {
    if (Platform.OS !== 'ios' || !enabled) {
      setKeyboardInset(0);
      return;
    }

    const show = ({endCoordinates}: KeyboardEvent) => {
      applyKeyboardInset(Math.max(0, endCoordinates.height - safeBottom));
    };
    const hide = () => applyKeyboardInset(0);
    const visibleKeyboard = Keyboard.metrics();
    if (visibleKeyboard) {
      applyKeyboardInset(Math.max(0, visibleKeyboard.height - safeBottom));
    }

    const showSubscription = Keyboard.addListener('keyboardWillShow', show);
    const hideSubscription = Keyboard.addListener('keyboardWillHide', hide);
    return () => {
      showSubscription.remove();
      hideSubscription.remove();
    };
  }, [applyKeyboardInset, enabled, safeBottom]);

  if (Platform.OS === 'ios') {
    return (
      <View style={[styles.content, enabled && {paddingBottom: keyboardInset}]}>
        {children}
      </View>
    );
  }

  return (
    <KeyboardAvoidingView
      style={styles.content}
      behavior="height"
      enabled={enabled}>
      {children}
    </KeyboardAvoidingView>
  );
}

function AppearanceBackground({uri}: {uri: string | null}) {
  const {appearance, palette, styles} = useAppTheme();
  const background = appearance.background;
  const treatment = background.treatment ?? 'clear';
  const resizeMode = mobileBackgroundResizeMode(background.fit);
  const opacity = Math.max(0, Math.min(1, background.image_opacity ?? 1));
  const overlayOpacity = Math.max(0, Math.min(1, background.overlay_opacity ?? 0));
  const blurRadius = Math.max(0, background.image_blur ?? 0) + (treatment === 'blur' ? 8 : 0);
  return (
    <View style={[StyleSheet.absoluteFill, {backgroundColor: palette.bg}]} pointerEvents="none">
      {uri ? (
        <View
          style={[
            StyleSheet.absoluteFill,
            treatment === 'mono' ? {filter: [{grayscale: 1}]} : null,
          ]}>
          <Image
            source={{uri}}
            style={[
              styles.backgroundImage,
              resizeMode === 'cover' && styles.backgroundImageCover,
              {opacity},
            ]}
            resizeMode={resizeMode}
            blurRadius={blurRadius}
          />
        </View>
      ) : null}
      {overlayOpacity > 0 ? (
        <View style={[StyleSheet.absoluteFill, {backgroundColor: background.overlay_color ?? '#000000', opacity: overlayOpacity}]} />
      ) : null}
      {treatment === 'dim' ? <View style={[StyleSheet.absoluteFill, styles.backgroundDim]} /> : null}
    </View>
  );
}

const stylesForMotion = StyleSheet.create({
  stage: {flex: 1},
});

mobilePerfMark('app.render.start');

type MilimController = ReturnType<typeof useMilimController>;
type ChatController = Pick<MilimController,
  | 'activeHost'
  | 'acceptedRetry'
  | 'pendingRetry'
  | 'bootstrap'
  | 'command'
  | 'draft'
  | 'execute'
  | 'loadMoreRunEvents'
  | 'loadRunDetails'
  | 'prepareAttachments'
  | 'refreshTimeline'
  | 'selectedThreadId'
  | 'setDraft'
  | 'status'
  | 'timeline'
>;
type HostsController = Pick<MilimController,
  | 'activeHost'
  | 'addManualHostCandidate'
  | 'hosts'
  | 'removeHost'
  | 'setActiveHost'
  | 'status'
>;

function App(): React.JSX.Element {
  const controller = useMilimController();
  const appearance = controller.bootstrap?.appearance;
  const theme = useMemo(
    () => appTheme(appearance),
    [appearance],
  );
  const {styles} = theme;
  const [screen, setScreen] = useState<AppScreen>('chat');
  const [threadDrawerVisible, setThreadDrawerVisible] = useState(false);
  const [pairingVisible, setPairingVisible] = useState(false);
  const [pairingClaim, setPairingClaim] = useState('');
  const pairedHostIds = useMemo(
    () => controller.hosts.map(host => host.hostId),
    [controller.hosts],
  );
  const openThreads = useCallback(() => setThreadDrawerVisible(true), []);
  const closeThreads = useCallback(() => setThreadDrawerVisible(false), []);
  const openPairing = useCallback(() => setPairingVisible(true), []);
  const returnToChat = useCallback(() => setScreen('chat'), []);
  const {setSelectedThreadId} = controller;
  const selectThread = useCallback((id: string) => {
    mobilePerfMark('thread.open.start');
    setSelectedThreadId(id);
    setScreen('chat');
    setThreadDrawerVisible(false);
  }, [setSelectedThreadId]);
  const chatController = useMemo<ChatController>(() => ({
    activeHost: controller.activeHost,
    acceptedRetry: controller.acceptedRetry,
    pendingRetry: controller.pendingRetry,
    bootstrap: controller.bootstrap,
    command: controller.command,
    draft: controller.draft,
    execute: controller.execute,
    loadMoreRunEvents: controller.loadMoreRunEvents,
    loadRunDetails: controller.loadRunDetails,
    prepareAttachments: controller.prepareAttachments,
    refreshTimeline: controller.refreshTimeline,
    selectedThreadId: controller.selectedThreadId,
    setDraft: controller.setDraft,
    status: controller.status,
    timeline: controller.timeline,
  }), [
    controller.activeHost,
    controller.acceptedRetry,
    controller.pendingRetry,
    controller.bootstrap,
    controller.command,
    controller.draft,
    controller.execute,
    controller.loadMoreRunEvents,
    controller.loadRunDetails,
    controller.prepareAttachments,
    controller.refreshTimeline,
    controller.selectedThreadId,
    controller.setDraft,
    controller.status,
    controller.timeline,
  ]);
  const hostsController = useMemo<HostsController>(() => ({
    activeHost: controller.activeHost,
    addManualHostCandidate: controller.addManualHostCandidate,
    hosts: controller.hosts,
    removeHost: controller.removeHost,
    setActiveHost: controller.setActiveHost,
    status: controller.status,
  }), [
    controller.activeHost,
    controller.addManualHostCandidate,
    controller.hosts,
    controller.removeHost,
    controller.setActiveHost,
    controller.status,
  ]);
  const threadDrawerEdgeGesture = useMemo(() => PanResponder.create({
    onMoveShouldSetPanResponderCapture: (_event, gestureState) => (
      !threadDrawerVisible &&
      gestureState.x0 <= DRAWER_EDGE_GESTURE_WIDTH &&
      gestureState.dx > DRAWER_SWIPE_ACTIVATION_DISTANCE &&
      Math.abs(gestureState.dx) > Math.abs(gestureState.dy) * 1.25
    ),
    onPanResponderRelease: (_event, gestureState) => {
      if (
        gestureState.dx >= DRAWER_SWIPE_COMMIT_DISTANCE ||
        gestureState.vx >= DRAWER_SWIPE_COMMIT_VELOCITY
      ) {
        openThreads();
      }
    },
  }), [openThreads, threadDrawerVisible]);

  useEffect(() => {
    mobilePerfMark('app.render.end');
    mobilePerfMeasure('app.render', 'app.render.start', 'app.render.end');
    void mobileStartupTiming();
    void cleanupStaleAttachments();
    void Linking.getInitialURL().then(url => {
      if (url) {
        setPairingClaim(url);
        setPairingVisible(true);
      }
    });
    const subscription = Linking.addEventListener('url', event => {
      setPairingClaim(event.url);
      setPairingVisible(true);
    });
    return () => subscription.remove();
  }, []);

  if (!controller.activeHost) {
    return (
      <AppThemeContext.Provider value={theme}>
        <SafeAreaProvider>
          <StatusBar barStyle={theme.isDark ? 'light-content' : 'dark-content'} />
          <PairingScreen
            claim={pairingClaim}
            setClaim={setPairingClaim}
            onPair={controller.pair}
            onPairNearby={controller.pairNearby}
            pairedHostIds={pairedHostIds}
          />
        </SafeAreaProvider>
      </AppThemeContext.Provider>
    );
  }

  const attentionCount =
    (controller.bootstrap?.pending_approvals.length ?? 0) +
    (controller.bootstrap?.queued_turns.length ?? 0);
  return (
    <AppThemeContext.Provider value={theme}>
      <SafeAreaProvider>
        <StatusBar barStyle={theme.isDark ? 'light-content' : 'dark-content'} />
        <View style={styles.root} {...threadDrawerEdgeGesture.panHandlers}>
        <AppearanceBackground uri={controller.appearanceBackgroundUri} />
        <SafeAreaView style={styles.app} edges={['top', 'right', 'bottom', 'left']}>
        <View style={styles.topbar}>
          <View style={styles.topbarLeading}>
            <MotionPressable
              style={styles.topbarButton}
              hitSlop={5}
              onPress={openThreads}
              accessibilityLabel="Open threads">
              <MilimIcon name="sidebar" size={17} color={theme.palette.secondary} />
            </MotionPressable>
            <MotionPressable
              style={[styles.brandGroup, screen === 'hosts' && styles.topbarDestinationActive]}
              onPress={() => setScreen('hosts')}
              hitSlop={4}
              accessibilityLabel="Open desktop hosts"
              accessibilityState={{selected: screen === 'hosts'}}>
              <Image source={milimLogo} style={styles.brandMark} />
              <View>
                <Text style={styles.brand}>milim</Text>
                <Text style={styles.hostLabel} numberOfLines={1}>
                  {lowercaseMilimBrand(controller.activeHost.displayName)}
                </Text>
              </View>
            </MotionPressable>
          </View>
          <View style={styles.topbarActions}>
            <ConnectionPill
              status={controller.status}
              count={attentionCount}
              active={screen === 'attention'}
              onPress={() => setScreen('attention')}
            />
          </View>
        </View>
        {controller.pendingRetry || controller.lastError ? (
          <Pressable
            style={styles.errorBanner}
            onPress={controller.pendingRetry
              ? () => void controller.retryPendingCommand().catch(showError)
              : controller.activeHost && controller.status === 'offline'
                ? controller.reconnect
                : undefined}>
            <Text style={styles.errorText} numberOfLines={2}>
              {controller.pendingRetry ? 'The desktop may have accepted your last command. Retry to confirm its result.' : controller.lastError}
            </Text>
            {controller.pendingRetry ? (
              <Text style={styles.retry}>{controller.status === 'online' ? 'Retry same command' : 'Reconnect and retry'}</Text>
            ) : controller.activeHost && controller.status === 'offline' ? (
              <Text style={styles.retry}>Retry connection</Text>
            ) : null}
          </Pressable>
        ) : null}
        <ThreadKeyboardAvoidingView enabled={screen === 'chat'}>
          <ScreenStage key={screen}>
          {screen === 'chat' ? (
            <ChatScreen controller={chatController} openThreads={openThreads} />
          ) : null}
          {screen === 'attention' ? (
            <AttentionScreen
              approvals={controller.bootstrap?.pending_approvals ?? []}
              queuedTurns={controller.bootstrap?.queued_turns ?? []}
              timeline={controller.timeline?.items ?? []}
              execute={controller.execute}
              onBack={() => setScreen('chat')}
            />
          ) : null}
          {screen === 'hosts' ? (
            <HostsScreen
              controller={hostsController}
              onPair={openPairing}
              onBack={returnToChat}
            />
          ) : null}
          </ScreenStage>
        </ThreadKeyboardAvoidingView>
        </SafeAreaView>
        </View>
        <ThreadDrawer
          visible={threadDrawerVisible}
          threads={controller.bootstrap?.threads ?? []}
          models={controller.bootstrap?.models ?? []}
          approvals={controller.bootstrap?.pending_approvals ?? []}
          selectedThreadId={controller.selectedThreadId}
          onSelect={selectThread}
          command={controller.command}
          execute={controller.execute}
          onClose={closeThreads}
        />
        <PairingModal
          visible={pairingVisible}
          claim={pairingClaim}
          setClaim={setPairingClaim}
          onClose={() => setPairingVisible(false)}
          onPair={async (claim, name) => {
            await controller.pair(claim, name);
            setPairingVisible(false);
          }}
          onPairNearby={async (host, name, signal, onStage) => {
            await controller.pairNearby(host, name, signal, onStage);
            setPairingVisible(false);
          }}
          pairedHostIds={pairedHostIds}
        />
      </SafeAreaProvider>
    </AppThemeContext.Provider>
  );
}

function ConnectionPill({
  status,
  count,
  active,
  onPress,
}: {
  status: string;
  count: number;
  active: boolean;
  onPress: () => void;
}) {
  const {styles} = useAppTheme();
  return (
    <MotionPressable
      style={[
        styles.connection,
        status === 'online' && styles.connectionOnline,
        active && styles.topbarDestinationActive,
      ]}
      onPress={onPress}
      hitSlop={5}
      accessibilityLabel={`Open Attention, ${status}${count ? `, ${count} pending` : ''}`}
      accessibilityState={{selected: active}}>
      <View style={[styles.dot, status === 'online' && styles.dotOnline]} />
      <Text style={styles.connectionText}>{status}</Text>
      {count ? <Text style={styles.connectionCount}>{count > 9 ? '9+' : count}</Text> : null}
    </MotionPressable>
  );
}

function PairingScreen({
  claim,
  setClaim,
  onPair,
  onPairNearby,
  pairedHostIds,
  additional = false,
  onClose,
}: {
  claim: string;
  setClaim: (value: string) => void;
  onPair: (claim: string, name: string) => Promise<unknown>;
  onPairNearby: (
    host: DiscoveredHost,
    deviceName: string,
    signal: AbortSignal,
    onStage: (stage: NearbyPairingStage) => void,
  ) => Promise<unknown>;
  pairedHostIds: readonly string[];
  additional?: boolean;
  onClose?: () => void;
}) {
  const {palette, styles} = useAppTheme();
  const [deviceName, setDeviceName] = useState(`${Platform.OS === 'ios' ? 'iPhone' : 'Android'} controller`);
  const [scanner, setScanner] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [nearbyHosts, setNearbyHosts] = useState<DiscoveredHost[]>([]);
  const [discovering, setDiscovering] = useState(true);
  const [discoveryError, setDiscoveryError] = useState<string | null>(null);
  const [showManualPairing, setShowManualPairing] = useState(Boolean(claim));
  const [nearbyPairing, setNearbyPairing] = useState<{
    host: DiscoveredHost;
    stage: NearbyPairingStage;
  } | null>(null);
  const discoveryVersion = useRef(0);
  const discoveryActive = useRef(false);
  const nearbyPairingAbort = useRef<AbortController | null>(null);
  const refreshNearbyHosts = useCallback(async () => {
    if (discoveryActive.current) return;
    discoveryActive.current = true;
    const version = ++discoveryVersion.current;
    setDiscovering(true);
    setDiscoveryError(null);
    try {
      const hosts = filterPairableMilimHosts(
        await discoverMilimHosts(),
        pairedHostIds,
      );
      if (version === discoveryVersion.current) setNearbyHosts(hosts);
    } catch (reason) {
      if (version === discoveryVersion.current) {
        setDiscoveryError(reason instanceof Error ? reason.message : String(reason));
      }
    } finally {
      if (version === discoveryVersion.current) {
        discoveryActive.current = false;
        setDiscovering(false);
      }
    }
  }, [pairedHostIds]);
  useEffect(() => {
    void refreshNearbyHosts();
    const timer = setInterval(() => void refreshNearbyHosts(), 15_000);
    return () => {
      clearInterval(timer);
      discoveryVersion.current += 1;
      discoveryActive.current = false;
    };
  }, [refreshNearbyHosts]);
  useEffect(() => {
    if (claim.trim()) setShowManualPairing(true);
  }, [claim]);
  useEffect(() => () => nearbyPairingAbort.current?.abort(), []);

  const connectNearby = async (host: DiscoveredHost) => {
    nearbyPairingAbort.current?.abort();
    const controller = new AbortController();
    nearbyPairingAbort.current = controller;
    setError(null);
    setNearbyPairing({host, stage: 'requesting'});
    try {
      await onPairNearby(host, deviceName, controller.signal, stage => {
        setNearbyPairing(current => current?.host.endpoint === host.endpoint
          ? {...current, stage}
          : current);
      });
    } catch (reason) {
      if (!controller.signal.aborted) {
        setError(friendlyPairingError(reason));
      }
    } finally {
      if (nearbyPairingAbort.current === controller) {
        nearbyPairingAbort.current = null;
        setNearbyPairing(null);
      }
    }
  };

  const cancelNearbyPairing = () => {
    nearbyPairingAbort.current?.abort();
    nearbyPairingAbort.current = null;
    setNearbyPairing(null);
  };
  const pair = async () => {
    setBusy(true);
    setError(null);
    try {
      await onPair(claim, deviceName);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  };
  return (
    <SafeAreaView style={styles.onboarding}>
      <View style={styles.onboardingTopbar}>
        <View style={styles.onboardingBrand}>
          <Image source={milimLogo} style={styles.onboardingMark} />
          <Text style={styles.onboardingWordmark}>milim</Text>
        </View>
        {onClose ? (
          <IconButton icon="x" label="Close pairing" onPress={onClose} />
        ) : (
          <Text style={styles.mobileLabel}>MOBILE</Text>
        )}
      </View>
      <ScrollView
        contentContainerStyle={styles.onboardingContent}
        keyboardShouldPersistTaps="handled">
        <View style={styles.onboardingIntro}>
          <Text style={styles.eyebrow}>{additional ? 'PAIR ANOTHER DESKTOP' : 'PAIR A DESKTOP'}</Text>
          <Text style={styles.heroTitle}>
            {additional ? 'Connect another milim workbench.' : 'Your milim workbench, in your pocket.'}
          </Text>
          <Text style={styles.heroCopy}>
            {additional
              ? 'Choose a nearby desktop, then approve the request there. Each desktop stays authoritative.'
              : 'Control the same threads, runs, queues, and approvals. Your desktop stays authoritative.'}
          </Text>
        </View>
        <View style={styles.nearbyPanel}>
          <View style={styles.nearbyHeading}>
            <View style={styles.flex}>
              <Text style={styles.nearbyTitle}>Nearby desktops</Text>
              <Text style={styles.nearbySubtitle}>
                {discovering
                  ? 'Searching your trusted network…'
                  : nearbyHosts.length
                    ? `${nearbyHosts.length} ready to pair`
                    : additional ? 'No new desktops detected' : 'No desktops detected'}
              </Text>
            </View>
            <MotionPressable
              style={[styles.nearbyRefresh, discovering && styles.disabled]}
              onPress={() => void refreshNearbyHosts()}
              disabled={discovering}
              accessibilityLabel="Refresh nearby desktops">
              <MilimIcon name="refresh" size={14} color={palette.secondary} />
            </MotionPressable>
          </View>
          {nearbyPairing ? (
            <View style={[styles.nearbyHost, styles.nearbyHostPending]}>
              <View style={styles.nearbyPairingSpinner}>
                <ActivityIndicator size="small" color={palette.text} />
              </View>
              <View style={styles.nearbyHostBody}>
                <Text style={styles.nearbyHostName} numberOfLines={1}>
                  {nearbyPairing.stage === 'requesting'
                    ? 'Requesting access…'
                    : nearbyPairing.stage === 'connecting'
                      ? 'Connecting securely…'
                      : 'Approve on your desktop'}
                </Text>
                <Text style={styles.nearbyHostEndpoint} numberOfLines={2}>
                  {nearbyPairing.stage === 'waiting'
                    ? `${lowercaseMilimBrand(nearbyPairing.host.name)} is waiting for your confirmation.`
                    : friendlyEndpoint(nearbyPairing.host.endpoint)}
                </Text>
              </View>
              <Button label="Cancel" tone="quiet" onPress={cancelNearbyPairing} />
            </View>
          ) : nearbyHosts.map(host => (
            <MotionPressable
              key={host.hostId ?? host.endpoint}
              style={styles.nearbyHost}
              onPress={() => void connectNearby(host)}
              accessibilityLabel={`Connect to ${lowercaseMilimBrand(host.name)}`}>
              <Image source={milimLogo} style={styles.nearbyHostMark} />
              <View style={styles.nearbyHostBody}>
                <Text style={styles.nearbyHostName} numberOfLines={1}>
                  {lowercaseMilimBrand(host.name)}
                </Text>
                <Text style={styles.nearbyHostEndpoint} numberOfLines={1}>
                  {friendlyEndpoint(host.endpoint)}
                </Text>
              </View>
              <View style={styles.nearbyConnectAction}>
                <Text style={styles.nearbyConnectText}>Connect</Text>
                <MilimIcon name="chevron-right" size={13} color={palette.text} />
              </View>
            </MotionPressable>
          ))}
          {!discovering && !nearbyHosts.length ? (
            <Text style={styles.nearbyEmpty}>
              {discoveryError
                ? 'Local discovery is unavailable. You can still pair with the QR code or link below.'
                : 'Enable the companion bridge and trusted-network discovery on your desktop. Some emulators and VPNs block local discovery.'}
            </Text>
          ) : null}
          {nearbyHosts.length ? (
            <Text style={styles.nearbyHelp}>
              Tap a desktop, then approve the request there. No code or link needed.
            </Text>
          ) : null}
          {error ? <Text style={styles.formError}>{error}</Text> : null}
        </View>
        <MotionPressable
          style={styles.manualPairToggle}
          onPress={() => setShowManualPairing(current => !current)}
          accessibilityLabel="Pair with QR code or link">
          <View style={styles.manualPairIcon}>
            <MilimIcon name="link" size={16} color={palette.secondary} />
          </View>
          <View style={styles.flex}>
            <Text style={styles.manualPairTitle}>Pair another way</Text>
            <Text style={styles.manualPairCopy}>Use a QR code or link when nearby discovery is unavailable.</Text>
          </View>
          <MilimIcon
            name={showManualPairing ? 'chevron-up' : 'chevron-down'}
            size={15}
            color={palette.muted}
          />
        </MotionPressable>
        {showManualPairing ? <View style={styles.pairingPanel}>
          <View style={styles.panelHeading}>
            <View style={styles.panelIcon}>
              <MilimIcon name="smartphone" size={17} color={palette.text} />
            </View>
            <View style={styles.flex}>
              <Text style={styles.panelTitle}>QR code or pairing link</Text>
              <Text style={styles.help}>For Tailscale, VPNs, or networks that block discovery</Text>
            </View>
          </View>
          <Text style={styles.fieldLabel}>DEVICE NAME</Text>
          <TextInput
        style={styles.input}
        value={deviceName}
        onChangeText={setDeviceName}
        placeholder="Device name"
        placeholderTextColor={palette.placeholder}
      />
      <Text style={styles.fieldLabel}>PAIRING LINK</Text>
      <TextInput
        style={[styles.input, styles.claimInput]}
        value={claim}
        onChangeText={setClaim}
        placeholder="Paste the link from milim desktop"
        placeholderTextColor={palette.placeholder}
        autoCapitalize="none"
        autoCorrect={false}
        multiline
      />
      <View style={styles.pairActions}>
        <Button label="Scan QR" icon="scan" tone="quiet" onPress={() => setScanner(true)} />
        <Button label={busy ? 'Pairing…' : 'Pair desktop'} icon="arrow-up" onPress={() => void pair()} disabled={busy || !claim.trim()} />
      </View>
        </View> : null}
        <View style={styles.directNote}>
          <View style={[styles.dot, styles.dotOnline]} />
          <Text style={styles.directNoteText}>
            Direct over Tailscale or trusted LAN. No milim account, cloud relay, or hosted transcript store.
          </Text>
        </View>
      </ScrollView>
      <Scanner visible={scanner} onClose={() => setScanner(false)} onRead={value => {
        setClaim(value);
        setScanner(false);
      }} />
    </SafeAreaView>
  );
}

function PairingModal({
  visible,
  claim,
  setClaim,
  onClose,
  onPair,
  onPairNearby,
  pairedHostIds,
}: {
  visible: boolean;
  claim: string;
  setClaim: (value: string) => void;
  onClose: () => void;
  onPair: (claim: string, deviceName: string) => Promise<unknown>;
  onPairNearby: (
    host: DiscoveredHost,
    deviceName: string,
    signal: AbortSignal,
    onStage: (stage: NearbyPairingStage) => void,
  ) => Promise<unknown>;
  pairedHostIds: readonly string[];
}) {
  return (
    <Modal visible={visible} animationType="slide" presentationStyle="pageSheet" onRequestClose={onClose}>
      {visible ? (
        <PairingScreen
          claim={claim}
          setClaim={setClaim}
          onPair={onPair}
          onPairNearby={onPairNearby}
          pairedHostIds={pairedHostIds}
          additional
          onClose={onClose}
        />
      ) : null}
    </Modal>
  );
}

function Scanner({visible, onClose, onRead}: {visible: boolean; onClose: () => void; onRead: (value: string) => void}) {
  const {palette, styles} = useAppTheme();
  const [allowed, setAllowed] = useState(Platform.OS === 'ios');
  useEffect(() => {
    if (!visible || Platform.OS !== 'android') return;
    void PermissionsAndroid.request(PermissionsAndroid.PERMISSIONS.CAMERA).then(result => {
      setAllowed(result === PermissionsAndroid.RESULTS.GRANTED);
    });
  }, [visible]);
  if (!visible) return null;
  const CameraView = require('react-native-camera-kit').Camera as typeof import('react-native-camera-kit').Camera;
  return (
    <Modal visible={visible} animationType="fade" onRequestClose={onClose}>
      <View style={styles.scanner}>
        {allowed ? (
          <CameraView
            style={StyleSheet.absoluteFill}
            scanBarcode
            showFrame
            frameColor={palette.accent}
            laserColor={palette.accent}
            onReadCode={event => onRead(event.nativeEvent.codeStringValue)}
          />
        ) : (
          <Text style={styles.formError}>Camera permission is needed only to scan the pairing QR.</Text>
        )}
        <SafeAreaView style={styles.scannerOverlay}>
          <Text style={styles.scannerTitle}>Scan milim pairing QR</Text>
          <Button label="Close" tone="quiet" onPress={onClose} />
        </SafeAreaView>
      </View>
    </Modal>
  );
}

const ThreadDrawer = React.memo(function MemoizedThreadDrawer({
  visible,
  threads,
  models,
  approvals,
  selectedThreadId,
  onSelect,
  command,
  execute,
  onClose,
}: {
  visible: boolean;
  threads: ThreadSummaryV1[];
  models: JsonValue[];
  approvals: PendingApprovalV1[];
  selectedThreadId: string | null;
  onSelect: (id: string) => void;
  command: ReturnType<typeof useMilimController>['command'];
  execute: ReturnType<typeof useMilimController>['execute'];
  onClose: () => void;
}) {
  const {palette, styles} = useAppTheme();
  const reduced = useReducedMotion();
  const {width} = useWindowDimensions();
  const drawerWidth = Math.min(width * 0.9, 380);
  const translateX = useRef(new Animated.Value(-drawerWidth)).current;
  const edgeOpacity = translateX.interpolate({
    inputRange: [-drawerWidth, 0],
    outputRange: [0, 1],
    extrapolate: 'clamp',
  });
  const [title, setTitle] = useState('');
  const [renaming, setRenaming] = useState<ThreadSummaryV1 | null>(null);
  const [actionsFor, setActionsFor] = useState<ThreadSummaryV1 | null>(null);
  const [renameTitle, setRenameTitle] = useState('');
  const [collapsedGroups, setCollapsedGroups] = useState<Record<string, boolean>>({});
  const [detailsFor, setDetailsFor] = useState<MobileThreadGroup | null>(null);
  const initializedGroups = useRef(false);
  const approvalCounts = useMemo(() => approvals.reduce<Record<string, number>>((counts, approval) => {
    counts[approval.thread_id] = (counts[approval.thread_id] ?? 0) + 1;
    return counts;
  }, {}), [approvals]);
  const groups = useMemo(() => groupMobileThreads(threads, approvalCounts), [approvalCounts, threads]);
  const visibleCount = groups.reduce((count, group) => count + group.threads.length, 0);
  const defaultModel = modelId(models[0]) ?? '';
  const drawerSections = useMemo(() => groups.map(group => ({
    group,
    data: collapsedGroups[group.id] ? [] : group.threads,
  })), [collapsedGroups, groups]);

  useEffect(() => {
    if (!visible || initializedGroups.current || !groups.length) return;
    const selectedGroup = groups.find(group => group.threads.some(thread => thread.id === selectedThreadId));
    const openGroupId = selectedGroup?.id ?? groups[0].id;
    setCollapsedGroups(Object.fromEntries(groups.map(group => [group.id, group.id !== openGroupId])));
    initializedGroups.current = true;
  }, [groups, selectedThreadId, visible]);

  useEffect(() => {
    if (!visible) return;
    translateX.setValue(reduced ? 0 : -drawerWidth);
    Animated.timing(translateX, {
      toValue: 0,
      duration: reduced ? 0 : 180,
      useNativeDriver: true,
    }).start();
  }, [drawerWidth, reduced, translateX, visible]);

  const close = useCallback(() => {
    setDetailsFor(null);
    if (reduced) {
      onClose();
      return;
    }
    Animated.timing(translateX, {
      toValue: -drawerWidth,
      duration: 150,
      useNativeDriver: true,
    }).start(({finished}) => {
      if (finished) onClose();
    });
  }, [drawerWidth, onClose, reduced, translateX]);

  const restoreDrawer = useCallback(() => {
    if (reduced) return;
    Animated.timing(translateX, {
      toValue: 0,
      duration: 120,
      useNativeDriver: true,
    }).start();
  }, [reduced, translateX]);

  const dismissGesture = useMemo(() => PanResponder.create({
    onMoveShouldSetPanResponderCapture: (_event, gestureState) => (
      gestureState.dx < -DRAWER_SWIPE_ACTIVATION_DISTANCE &&
      Math.abs(gestureState.dx) > Math.abs(gestureState.dy) * 1.25
    ),
    onPanResponderMove: (_event, gestureState) => {
      if (!reduced) translateX.setValue(Math.max(-drawerWidth, Math.min(0, gestureState.dx)));
    },
    onPanResponderRelease: (_event, gestureState) => {
      if (
        gestureState.dx <= -DRAWER_SWIPE_COMMIT_DISTANCE ||
        gestureState.vx <= -DRAWER_SWIPE_COMMIT_VELOCITY
      ) {
        close();
      } else {
        restoreDrawer();
      }
    },
    onPanResponderTerminate: restoreDrawer,
  }), [close, drawerWidth, reduced, restoreDrawer, translateX]);

  const create = async () => {
    const result = await command('thread.create', {
      title: title.trim() || 'New chat',
      settings: {model: defaultModel, privacy: 'off', toolApproval: 'review'},
    }, null);
    setTitle('');
    if (result.thread_id) onSelect(result.thread_id);
  };

  return (
    <Modal visible={visible} transparent animationType="none" onRequestClose={close} statusBarTranslucent>
      <SafeAreaProvider>
      <View style={styles.drawerBackdrop}>
        <Pressable style={styles.drawerDismiss} onPress={close} accessibilityRole="button" accessibilityLabel="Close threads" />
        <Animated.View
          style={[styles.drawer, {width: drawerWidth, transform: [{translateX}]}]}
          {...dismissGesture.panHandlers}>
          <Animated.View pointerEvents="none" style={[styles.drawerEdgeFade, {opacity: edgeOpacity}]}>
            <DrawerBackdropFade />
          </Animated.View>
          <SafeAreaView style={styles.drawerSafe} edges={['top', 'left', 'bottom']}>
            <View style={styles.drawerHeader}>
              <View style={styles.drawerHeading}>
                <Text style={styles.eyebrow}>THREADS</Text>
                <View style={styles.drawerTitleRow}>
                  <Text style={styles.drawerTitle}>Workspace</Text>
                  <Text style={styles.drawerCount}>{visibleCount}</Text>
                </View>
              </View>
              <MotionPressable style={styles.drawerClose} onPress={close} accessibilityLabel="Close threads">
                <MilimIcon name="x" size={17} color={palette.secondary} />
              </MotionPressable>
            </View>
            <View style={styles.drawerCreateRow}>
              <TextInput
                style={[styles.input, styles.flex]}
                value={title}
                onChangeText={setTitle}
                placeholder="New chat"
                placeholderTextColor={palette.placeholder}
                returnKeyType="done"
                onSubmitEditing={() => void create().catch(showError)}
              />
              <MotionPressable
                style={[styles.drawerCreateButton, !defaultModel && styles.disabled]}
                disabled={!defaultModel}
                onPress={() => void create().catch(showError)}
                accessibilityLabel="Create thread">
                <MilimIcon name="plus" size={17} color={palette.accentInk} />
              </MotionPressable>
            </View>
            <SectionList
              style={styles.drawerGroups}
              contentContainerStyle={styles.drawerGroupList}
              sections={drawerSections}
              keyExtractor={item => item.id}
              stickySectionHeadersEnabled={false}
              initialNumToRender={18}
              windowSize={7}
              renderSectionFooter={() => <View style={styles.drawerGroupSeparator} />}
              ListEmptyComponent={<Empty title="No threads yet" copy="Create one here or start from an Agent on desktop." />}
              renderSectionHeader={({section}) => {
                const group = section.group;
                const collapsed = Boolean(collapsedGroups[group.id]);
                return (
                  <View style={styles.drawerGroup}>
                    <View style={styles.drawerGroupHeader}>
                      <MotionPressable
                        style={styles.drawerGroupToggle}
                        onPress={() => setCollapsedGroups(current => ({...current, [group.id]: !collapsed}))}
                        accessibilityLabel={`${collapsed ? 'Expand' : 'Collapse'} ${group.label}`}>
                        <MilimIcon name={collapsed ? 'chevron-down' : 'chevron-up'} size={13} color={palette.muted} />
                        <View style={styles.drawerGroupIcon}>
                          <MilimIcon name={group.workspace ? 'folder' : 'sparkles'} size={14} color={palette.secondary} />
                        </View>
                        <View style={styles.drawerGroupBody}>
                          <Text style={styles.drawerGroupTitle} numberOfLines={1}>{group.label}</Text>
                          <Text style={styles.drawerGroupSubtitle}>{group.subtitle} · {group.threads.length}</Text>
                        </View>
                        {group.busy ? <View style={[styles.dot, styles.dotOnline]} /> : null}
                        {group.attentionCount ? <Text style={styles.drawerAttention}>{group.attentionCount}</Text> : null}
                      </MotionPressable>
                      {group.workspace ? (
                        <MotionPressable
                          style={styles.drawerGroupInfo}
                          onPress={() => setDetailsFor(current => current?.id === group.id ? null : group)}
                          accessibilityLabel={`Project details for ${group.label}`}>
                          <MilimIcon name="more-horizontal" size={16} color={palette.muted} />
                        </MotionPressable>
                      ) : null}
                    </View>
                    {detailsFor?.id === group.id ? (
                      <View style={styles.drawerProjectDetails}>
                        <Text style={styles.drawerProjectLabel}>PROJECT LOCATION</Text>
                        <Text style={styles.drawerProjectPath} selectable>{group.workspace}</Text>
                      </View>
                    ) : null}
                  </View>
                );
              }}
              renderItem={({item}) => (
                <ThreadCard
                  thread={item}
                  attentionCount={item.queued_turns + (approvalCounts[item.id] ?? 0)}
                  selected={item.id === selectedThreadId}
                  onOpen={() => onSelect(item.id)}
                  onMenu={() => setActionsFor(item)}
                />
              )}
            />
          </SafeAreaView>
        </Animated.View>
      </View>
      <PickerSheetFrame
        visible={Boolean(actionsFor)}
        title={actionsFor?.title ?? 'Thread'}
        subtitle={actionsFor?.workspace || 'No project'}
        compact
        onClose={() => setActionsFor(null)}>
        <View style={styles.actionSheetList}>
          <ActionSheetButton icon="pencil" label="Rename" onPress={() => {
            if (!actionsFor) return;
            setRenaming(actionsFor);
            setRenameTitle(actionsFor.title);
            setActionsFor(null);
          }} />
          <ActionSheetButton icon="archive" label="Archive" onPress={() => {
            if (!actionsFor) return;
            const target = actionsFor;
            setActionsFor(null);
            void command('thread.archive', {archived: true}, target.id, target.revision).catch(showError);
          }} />
          <ActionSheetButton icon="trash" label="Delete" danger onPress={() => {
            if (!actionsFor) return;
            const target = actionsFor;
            setActionsFor(null);
            void confirmDestructive(execute, {
              command_id: newCommandId(),
              kind: 'thread.delete',
              thread_id: target.id,
              expected_revision: target.revision,
              payload: null,
            });
          }} />
        </View>
      </PickerSheetFrame>
      <Modal visible={Boolean(renaming)} transparent animationType="fade" onRequestClose={() => setRenaming(null)}>
        <View style={styles.dialogBackdrop}>
          <View style={styles.dialogCard}>
            <Text style={styles.attentionTitle}>Rename thread</Text>
            <TextInput
              style={styles.input}
              value={renameTitle}
              onChangeText={setRenameTitle}
              autoFocus
              maxLength={160}
              placeholder="Thread title"
              placeholderTextColor={palette.placeholder}
            />
            <View style={styles.actionRow}>
              <Button label="Cancel" tone="quiet" onPress={() => setRenaming(null)} />
              <Button
                label="Save"
                disabled={!renameTitle.trim()}
                onPress={() => {
                  if (!renaming) return;
                  void command('thread.rename', {title: renameTitle.trim()}, renaming.id, renaming.revision)
                    .then(() => setRenaming(null))
                    .catch(showError);
                }}
              />
            </View>
          </View>
        </View>
      </Modal>
      </SafeAreaProvider>
    </Modal>
  );
});

function ThreadCard({thread, attentionCount, selected, onOpen, onMenu}: {thread: ThreadSummaryV1; attentionCount: number; selected: boolean; onOpen: () => void; onMenu: () => void}) {
  const {palette, styles} = useAppTheme();
  return (
    <Pressable style={[styles.threadCard, selected && styles.threadCardSelected]} onPress={onOpen}>
      <View style={styles.threadTopline}>
        <Text style={styles.threadTitle} numberOfLines={1}>{thread.title}</Text>
        {thread.origin?.kind === 'schedule' ? <Text style={styles.threadOrigin}>Scheduled</Text> : null}
        {attentionCount ? <Text style={styles.queued}>{attentionCount}</Text> : null}
      </View>
      <Pressable
        style={styles.threadMenu}
        hitSlop={8}
        accessibilityRole="button"
        accessibilityLabel={`Actions for ${thread.title}`}
        onPress={event => {
          event.stopPropagation();
          onMenu();
        }}>
        <MilimIcon name="more-horizontal" size={17} color={palette.muted} />
      </Pressable>
    </Pressable>
  );
}

const ChatScreen = React.memo(function MemoizedChatScreen({controller, openThreads}: {controller: ChatController; openThreads: () => void}) {
  const {markdownStyles, palette, styles} = useAppTheme();
  const reduced = useReducedMotion();
  const thread = controller.bootstrap?.threads.find(item => item.id === controller.selectedThreadId);
  useEffect(() => {
    if (!thread || controller.timeline?.threadId !== thread.id) return;
    mobilePerfMark('thread.open.end');
    mobilePerfMeasure('thread.open', 'thread.open.start', 'thread.open.end');
  }, [controller.timeline?.threadId, thread]);
  const threadApprovals = useMemo(
    () => controller.bootstrap?.pending_approvals.filter(approval => approval.thread_id === thread?.id) ?? [],
    [controller.bootstrap?.pending_approvals, thread?.id],
  );
  const threadPendingInputs = useMemo(
    () => controller.bootstrap?.pending_inputs.filter(input => input.thread_id === thread?.id) ?? [],
    [controller.bootstrap?.pending_inputs, thread?.id],
  );
  const transcriptProjection = useRef<{
    threadId: string | null;
    epoch: string;
    cache: TranscriptProjectionCache | null;
  }>({threadId: null, epoch: '', cache: null});
  const transcriptItems = useMemo(() => {
    mobilePerfMark('transcript.project.start');
    const timeline = controller.timeline;
    const threadId = thread?.id ?? null;
    const matchingTimeline = timeline?.threadId === threadId ? timeline : null;
    const epoch = matchingTimeline?.epoch ?? '';
    const previous = transcriptProjection.current;
    const cache = projectTranscriptIncrementally(
      previous.threadId === threadId && previous.epoch === epoch ? previous.cache : null,
      matchingTimeline?.items ?? [],
      threadApprovals,
      threadPendingInputs,
    );
    mobilePerfMark('transcript.project.end');
    mobilePerfMeasure('transcript.project', 'transcript.project.start', 'transcript.project.end');
    transcriptProjection.current = {threadId, epoch, cache};
    return cache.projected;
  }, [controller.timeline, thread?.id, threadApprovals, threadPendingInputs]);
  const inspectableRunIds = useMemo(
    () => new Set(
      transcriptItems
        .filter(item => item.kind === 'message' && item.role === 'assistant' && item.ledgerVersion === 1 && item.runId)
        .map(item => (item as Extract<ProjectedTranscriptItem, {kind: 'message'}>).runId as string),
    ),
    [transcriptItems],
  );
  const activeRun = useMemo(
    () => controller.bootstrap?.active_runs.find(run => run.thread_id === thread?.id) ?? null,
    [controller.bootstrap?.active_runs, thread?.id],
  );
  const [attachments, setAttachments] = useState<ControlAttachmentV1[]>([]);
  const handledRetry = useRef<string | null>(null);
  useEffect(() => {
    const accepted = controller.acceptedRetry;
    if (!accepted || !['turn.send', 'turn.steer'].includes(accepted.command.kind) ||
      accepted.hostId !== controller.activeHost?.hostId ||
      accepted.command.thread_id !== thread?.id || handledRetry.current === accepted.command.command_id) return;
    handledRetry.current = accepted.command.command_id;
    const payload = accepted.command.payload as {attachments?: {id: string}[]};
    const sentIds = new Set(payload.attachments?.map(item => item.id) ?? []);
    const sent = attachments.filter(item => sentIds.has(item.id));
    if (sent.length) {
      void cleanupAttachments(sent).catch(showError);
      setAttachments(current => current.filter(item => !sentIds.has(item.id)));
    }
  }, [attachments, controller.acceptedRetry, controller.activeHost?.hostId, thread?.id]);
  const [busy, setBusy] = useState(false);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [modelPickerVisible, setModelPickerVisible] = useState(false);
  const [agentPickerVisible, setAgentPickerVisible] = useState(false);
  const [attachmentMenuVisible, setAttachmentMenuVisible] = useState(false);
  const [showLatest, setShowLatest] = useState(false);
  const [awayFromLatest, setAwayFromLatest] = useState(false);
  const [inputFocused, setInputFocused] = useState(false);
  const [forcedComposerOpen, setForcedComposerOpen] = useState(false);
  const [composerCompact, setComposerCompact] = useState(true);
  const [expandedComposerHeight, setExpandedComposerHeight] = useState(138);
  const [compactComposerHeight, setCompactComposerHeight] = useState(56);
  const messageList = useRef<FlatList<ProjectedTranscriptItem>>(null);
  const composerInput = useRef<React.ElementRef<typeof TextInput>>(null);
  const composerProgress = useRef(new Animated.Value(0)).current;
  const shouldScrollToLatest = useRef(true);
  const followingLatest = useRef(true);
  const returningToLatest = useRef(false);
  const latestScrollFrame = useRef<number | null>(null);
  const latestScrollAnimated = useRef(false);
  const showLatestRef = useRef(false);
  const awayFromLatestRef = useRef(false);
  const scheduleLatestScroll = useCallback((animated: boolean) => {
    latestScrollAnimated.current = latestScrollFrame.current === null
      ? animated
      : latestScrollAnimated.current && animated;
    if (latestScrollFrame.current !== null) return;
    latestScrollFrame.current = requestAnimationFrame(() => {
      latestScrollFrame.current = null;
      const shouldAnimate = latestScrollAnimated.current;
      latestScrollAnimated.current = false;
      messageList.current?.scrollToEnd({animated: shouldAnimate});
    });
  }, []);
  const updateShowLatest = useCallback((value: boolean) => {
    if (showLatestRef.current === value) return;
    showLatestRef.current = value;
    setShowLatest(value);
  }, []);
  const updateAwayFromLatest = useCallback((value: boolean) => {
    if (awayFromLatestRef.current === value) return;
    awayFromLatestRef.current = value;
    setAwayFromLatest(value);
  }, []);
  const modelsForPicker = useMemo(() => {
    const models = [...(controller.bootstrap?.models ?? [])];
    const ids = new Set(mobileModelOptions(models).map(model => model.id));
    for (const summary of controller.bootstrap?.threads ?? []) {
      if (summary.model && !ids.has(summary.model)) {
        models.push({id: summary.model, owned_by: 'milim'});
        ids.add(summary.model);
      }
    }
    return models;
  }, [controller.bootstrap?.models, controller.bootstrap?.threads]);
  const selectedModel = useMemo(
    () => mobileModelOptions(modelsForPicker)
      .find(model => model.id === thread?.model) ?? null,
    [modelsForPicker, thread?.model],
  );
  const hasPendingApproval = threadApprovals.length > 0;
  const shouldCompactComposer = canUseCompactComposer({
    awayFromLatest,
    draft: controller.draft,
    attachmentCount: attachments.length,
    inputFocused,
    pendingApproval: hasPendingApproval,
    forcedOpen: forcedComposerOpen,
  });
  const composerHeight = composerProgress.interpolate({
    inputRange: [0, 1],
    outputRange: [expandedComposerHeight, compactComposerHeight],
  });
  const transcriptBottomInset = Animated.add(composerHeight, TRANSCRIPT_FADE_HEIGHT);

  useEffect(() => {
    setComposerCompact(shouldCompactComposer);
    if (reduced) {
      composerProgress.setValue(shouldCompactComposer ? 1 : 0);
      return;
    }
    const animation = Animated.timing(composerProgress, {
      toValue: shouldCompactComposer ? 1 : 0,
      duration: 160,
      useNativeDriver: false,
    });
    animation.start();
    return () => animation.stop();
  }, [composerProgress, reduced, shouldCompactComposer]);

  useEffect(() => {
    if (!forcedComposerOpen || composerCompact) return;
    const focusTimer = setTimeout(() => {
      composerInput.current?.focus();
    }, reduced ? 0 : 160);
    return () => clearTimeout(focusTimer);
  }, [composerCompact, forcedComposerOpen, reduced]);

  useEffect(() => {
    if ((!shouldScrollToLatest.current && !followingLatest.current) || !transcriptItems.length) return;
    const animated = !shouldScrollToLatest.current;
    shouldScrollToLatest.current = false;
    scheduleLatestScroll(animated);
  }, [scheduleLatestScroll, transcriptItems]);

  useEffect(() => {
    const keepLatestVisible = () => {
      if (!followingLatest.current) return;
      scheduleLatestScroll(false);
    };
    const shown = Keyboard.addListener('keyboardDidShow', keepLatestVisible);
    const hidden = Keyboard.addListener('keyboardDidHide', keepLatestVisible);
    return () => {
      shown.remove();
      hidden.remove();
    };
  }, [scheduleLatestScroll]);

  useEffect(() => () => {
    if (latestScrollFrame.current !== null) cancelAnimationFrame(latestScrollFrame.current);
  }, []);

  useEffect(() => {
    shouldScrollToLatest.current = true;
    followingLatest.current = true;
    returningToLatest.current = false;
    updateShowLatest(false);
    updateAwayFromLatest(false);
    setForcedComposerOpen(false);
    setInputFocused(false);
  }, [thread?.id, updateAwayFromLatest, updateShowLatest]);
  const renderTranscriptItem = useCallback(({item}: {item: ProjectedTranscriptItem}) => (
    <TranscriptItemView
      item={item}
      markdownStyles={markdownStyles}
      execute={controller.execute}
      runDetailsEnabled={item.kind === 'activity' && inspectableRunIds.has(item.runId)}
      loadRunDetails={controller.loadRunDetails}
      loadMoreRunEvents={controller.loadMoreRunEvents}
    />
  ), [
    controller.execute,
    controller.loadMoreRunEvents,
    controller.loadRunDetails,
    inspectableRunIds,
    markdownStyles,
  ]);
  if (!thread) {
    return <Empty title="Choose a thread" copy="Open the thread drawer to select or create a conversation." action="Open threads" onAction={openThreads} />;
  }
  const missingAgent = Boolean(
    thread.agent_id && !controller.bootstrap?.agents.some(agent => agent.id === thread.agent_id),
  );
  const activeAgent = controller.bootstrap?.agents.find(agent => agent.id === thread.agent_id) ?? null;
  const send = async () => {
    if (controller.status !== 'online') {
      Alert.alert('Saved as a draft', 'milim will not auto-send this prompt after reconnecting.');
      return;
    }
    setBusy(true);
    try {
      const wireAttachments = await controller.prepareAttachments(attachments);
      await controller.command(
        'turn.send',
        {
          text: promptWithAttachments(controller.draft, attachments),
          display_text: controller.draft,
          attachments: wireAttachments,
        } as unknown as JsonValue,
        thread.id,
        thread.revision,
      );
      controller.setDraft('');
      await cleanupAttachments(attachments);
      setAttachments([]);
      shouldScrollToLatest.current = true;
      followingLatest.current = true;
      returningToLatest.current = false;
      updateShowLatest(false);
      updateAwayFromLatest(false);
      scheduleLatestScroll(!reduced);
    } catch (error) {
      showError(error);
    } finally {
      setBusy(false);
    }
  };
  const steer = async () => {
    if (!activeRun?.capabilities.steering) return;
    if (controller.status !== 'online') {
      Alert.alert('Desktop offline', 'Steering requires the active desktop run.');
      return;
    }
    setBusy(true);
    try {
      const wireAttachments = await controller.prepareAttachments(attachments);
      await controller.command(
        'turn.steer',
        {
          run_id: activeRun.id,
          text: promptWithAttachments(controller.draft, attachments),
          display_text: controller.draft,
          attachments: wireAttachments,
        } as unknown as JsonValue,
        thread.id,
      );
      controller.setDraft('');
      await cleanupAttachments(attachments);
      setAttachments([]);
    } catch (error) {
      showError(error);
    } finally {
      setBusy(false);
    }
  };
  const loadOlder = async () => {
    setLoadingOlder(true);
    try {
      await controller.refreshTimeline('before');
    } catch (error) {
      showError(error);
    } finally {
      setLoadingOlder(false);
    }
  };
  const addAttachments = async (source: 'library' | 'camera' | 'file') => {
    setAttachmentMenuVisible(false);
    const items = source === 'file' ? await pickFiles() : await pickPhoto(source);
    setAttachments(current => [...current, ...items].slice(0, MAX_ATTACHMENTS));
  };
  const removeAttachment = async (attachment: ControlAttachmentV1) => {
    await cleanupAttachments([attachment]);
    setAttachments(current => current.filter(item => item.id !== attachment.id));
  };
  return (
    <View style={styles.screen}>
      <View style={styles.chatHeader}>
        <Text style={styles.chatTitle} numberOfLines={1}>{thread.title}</Text>
        {thread.origin?.kind === 'schedule' ? <Text style={styles.threadOrigin}>Scheduled</Text> : null}
        {thread.busy ? (
          <View style={styles.chatRunState}>
            <View style={[styles.dot, styles.dotOnline]} />
            <Text style={styles.chatRunText}>RUNNING</Text>
          </View>
        ) : null}
      </View>
      <View style={styles.chatBody}>
        <View style={styles.transcriptMask}>
        <FlatList
          ref={messageList}
          style={styles.messageList}
          contentContainerStyle={styles.messageContent}
          data={transcriptItems}
          keyExtractor={item => item.id}
          windowSize={7}
          maxToRenderPerBatch={6}
          updateCellsBatchingPeriod={32}
          ListHeaderComponent={controller.timeline?.hasOlder ? (
            <MotionPressable
              style={[styles.historyControl, loadingOlder && styles.disabled]}
              disabled={loadingOlder}
              onPress={() => void loadOlder()}>
              <MilimIcon name="refresh" size={13} color={palette.secondary} />
              <Text style={styles.historyText}>{loadingOlder ? 'Loading transcript…' : 'Load earlier messages'}</Text>
            </MotionPressable>
          ) : undefined}
          ListEmptyComponent={<Empty title="Ready when you are" copy="Runs continue on the desktop process even if this screen disconnects." />}
          ListFooterComponent={<Animated.View style={{height: transcriptBottomInset}} />}
          maintainVisibleContentPosition={{minIndexForVisible: 0}}
          scrollEventThrottle={32}
          onScrollBeginDrag={() => {
            returningToLatest.current = false;
          }}
          onContentSizeChange={() => {
            if (!followingLatest.current || !transcriptItems.length) return;
            scheduleLatestScroll(false);
          }}
          onScroll={({nativeEvent}) => {
            const distance = transcriptDistanceFromLatest({
              contentHeight: nativeEvent.contentSize.height,
              viewportHeight: nativeEvent.layoutMeasurement.height,
              offsetY: nativeEvent.contentOffset.y,
              bottomInset: (composerCompact ? compactComposerHeight : expandedComposerHeight) + TRANSCRIPT_FADE_HEIGHT,
            });
            const nearLatest = !shouldHoldCompactComposerForLatestReturn(distance);
            if (returningToLatest.current) {
              if (!nearLatest) return;
              returningToLatest.current = false;
              followingLatest.current = true;
              updateShowLatest(false);
              updateAwayFromLatest(false);
              return;
            }
            followingLatest.current = nearLatest;
            updateShowLatest(!nearLatest);
            updateAwayFromLatest(nextAwayFromLatest(awayFromLatestRef.current, distance));
          }}
          renderItem={renderTranscriptItem}
        />
        <TranscriptFadeOverlay bottomInset={composerHeight} color={palette.bg} />
        </View>
        {showLatest ? (
          <Animated.View pointerEvents="box-none" style={[styles.latestDock, {bottom: composerHeight}]}>
            <MotionPressable
              style={styles.latestButton}
              onPress={() => {
                returningToLatest.current = !reduced;
                followingLatest.current = true;
                updateShowLatest(false);
                scheduleLatestScroll(!reduced);
                if (reduced) updateAwayFromLatest(false);
              }}>
              <MilimIcon name="chevron-down" size={13} color={palette.secondary} />
              <Text style={styles.latestText}>Latest</Text>
            </MotionPressable>
          </Animated.View>
        ) : null}
      <Animated.View
        style={[
          styles.composerDock,
          {height: composerHeight},
        ]}>
        <Animated.View
          style={[
            styles.composerLayer,
            composerCompact && styles.composerLayerFront,
            {
              opacity: composerProgress,
              transform: [{
                translateY: composerProgress.interpolate({inputRange: [0, 1], outputRange: [6, 0]}),
              }],
            },
          ]}
          pointerEvents={composerCompact ? 'auto' : 'none'}
          accessibilityElementsHidden={!composerCompact}
          importantForAccessibility={composerCompact ? 'auto' : 'no-hide-descendants'}
          onLayout={({nativeEvent}) => setCompactComposerHeight(nativeEvent.layout.height)}>
        {missingAgent ? <Text style={styles.missing}>This Agent was deleted. Clear or replace it before sending.</Text> : null}
        <View style={styles.compactComposer}>
          <MotionPressable
            style={styles.compactComposerPrompt}
            hitSlop={3}
            onPress={() => {
              setForcedComposerOpen(true);
            }}
            accessibilityLabel="Expand message composer">
            {thread.busy ? <View style={[styles.dot, styles.dotOnline]} /> : <MilimIcon name="sparkles" size={13} color={palette.muted} />}
            <Text style={styles.compactComposerText} numberOfLines={1}>
              {thread.busy ? 'milim is working · Message milim…' : 'Message milim…'}
            </Text>
          </MotionPressable>
          {thread.busy ? (
            <IconButton icon="square" label="Stop generating" tone="quiet" onPress={() => void controller.command('turn.stop', null, thread.id).catch(showError)} />
          ) : null}
        </View>
        </Animated.View>
        <Animated.View
          style={[
            styles.composerLayer,
            !composerCompact && styles.composerLayerFront,
            {
              opacity: composerProgress.interpolate({inputRange: [0, 1], outputRange: [1, 0]}),
              transform: [{
                translateY: composerProgress.interpolate({inputRange: [0, 1], outputRange: [0, 8]}),
              }],
            },
          ]}
          pointerEvents={composerCompact ? 'none' : 'auto'}
          accessibilityElementsHidden={composerCompact}
          importantForAccessibility={composerCompact ? 'no-hide-descendants' : 'auto'}
          onLayout={({nativeEvent}) => setExpandedComposerHeight(nativeEvent.layout.height)}>
        {missingAgent ? <Text style={styles.missing}>This Agent was deleted. Clear or replace it before sending.</Text> : null}
        <View style={styles.composer}>
        <View style={styles.composerContextRow}>
          <PickerChip
            icon="sparkles"
            providerBrand={selectedModel?.brand ?? null}
            label={selectedModel?.label || thread.model || 'Choose model'}
            onPress={() => setModelPickerVisible(true)}
          />
          <PickerChip
            icon="bolt"
            leading={activeAgent ? <AgentAvatar {...activeAgent} size={14} /> : undefined}
            label={missingAgent ? 'Missing Agent' : activeAgent?.name || 'No Agent'}
            warning={missingAgent}
            onPress={() => setAgentPickerVisible(true)}
          />
        </View>
        {attachments.length ? (
          <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={styles.attachments}>
            {attachments.map(attachment => (
              <MotionPressable
                key={attachment.id}
                style={styles.attachment}
                accessibilityLabel={`Remove ${attachment.name}`}
                onPress={() => void removeAttachment(attachment).catch(showError)}>
                <MilimIcon name={attachment.mime.startsWith('image/') ? 'image' : 'file'} size={12} color={palette.secondary} />
                <Text style={styles.attachmentText} numberOfLines={1}>{attachment.name}</Text>
                <MilimIcon name="x" size={11} color={palette.muted} />
              </MotionPressable>
            ))}
          </ScrollView>
        ) : null}
        <TextInput
          ref={composerInput}
          style={styles.composerInput}
          value={controller.draft}
          onChangeText={controller.setDraft}
          placeholder={thread.busy ? 'Queue another turn…' : 'Message milim…'}
          placeholderTextColor={palette.placeholder}
          multiline
          maxLength={32_000}
          onFocus={() => {
            setInputFocused(true);
            setForcedComposerOpen(true);
          }}
          onBlur={() => {
            setInputFocused(false);
            setForcedComposerOpen(false);
          }}
        />
        <View style={styles.composerActions}>
          <IconButton
            icon="paperclip"
            label="Add attachment"
            onPress={() => setAttachmentMenuVisible(true)}
          />
          <View style={styles.composerSpacer} />
          {thread.busy ? (
            <>
              {activeRun?.capabilities.steering ? (
                <IconButton
                  icon="bolt"
                  label="Steer next step"
                  tone="quiet"
                  disabled={busy || Boolean(controller.pendingRetry) || (!controller.draft.trim() && !attachments.length)}
                  onPress={() => void steer()}
                />
              ) : null}
              <IconButton icon="square" label="Stop generating" tone="quiet" onPress={() => void controller.command('turn.stop', null, thread.id).catch(showError)} />
            </>
          ) : (
            <IconButton icon="refresh" label="Regenerate" onPress={() => void controller.command('turn.regenerate', null, thread.id, thread.revision).catch(showError)} />
          )}
          <IconButton
            icon="arrow-up"
            label={busy ? 'Sending' : thread.busy ? 'Queue message' : 'Send message'}
            tone="accent"
            disabled={busy || Boolean(controller.pendingRetry) || missingAgent || (!controller.draft.trim() && !attachments.length)}
            onPress={() => void send()}
          />
        </View>
      </View>
        </Animated.View>
      </Animated.View>
      </View>
      <PickerSheetFrame
        visible={attachmentMenuVisible}
        title="Add attachment"
        subtitle={`${attachments.length} of ${MAX_ATTACHMENTS} attached`}
        compact
        onClose={() => setAttachmentMenuVisible(false)}>
        <View style={styles.actionSheetList}>
          <ActionSheetButton icon="camera" label="Take photo" onPress={() => void addAttachments('camera').catch(showError)} />
          <ActionSheetButton icon="image" label="Photo library" onPress={() => void addAttachments('library').catch(showError)} />
          <ActionSheetButton icon="paperclip" label="Choose file" onPress={() => void addAttachments('file').catch(showError)} />
        </View>
      </PickerSheetFrame>
      <ModelPickerSheet
        visible={modelPickerVisible}
        hostId={controller.activeHost?.hostId ?? controller.bootstrap?.host_id ?? ''}
        models={modelsForPicker}
        favoriteIds={controller.bootstrap?.capabilities.model_favorites === true
          ? controller.bootstrap.favorite_model_ids ?? []
          : undefined}
        selectedId={thread.model}
        reasoningEffortOverrides={thread.reasoning_effort_overrides}
        onClose={() => setModelPickerVisible(false)}
        onFavoriteIdsChange={controller.bootstrap?.capabilities.model_favorites === true
          ? async favoriteModelIds => {
              await controller.command(
                'model_favorites.set',
                {favorite_model_ids: favoriteModelIds},
                null,
              );
            }
          : undefined}
        onSelect={(id, reasoningEffort) => {
          setModelPickerVisible(false);
          void controller.command(
            'thread.set_model',
            {model: id, ...(reasoningEffort ? {reasoning_effort: reasoningEffort} : {})},
            thread.id,
            thread.revision,
          ).catch(showError);
        }}
      />
      <AgentPickerSheet
        visible={agentPickerVisible}
        agents={controller.bootstrap?.agents ?? []}
        selectedId={thread.agent_id}
        onClose={() => setAgentPickerVisible(false)}
        onSelect={id => {
          setAgentPickerVisible(false);
          void controller.command('thread.set_agent', {agent_id: id || null}, thread.id, thread.revision).catch(showError);
        }}
      />
    </View>
  );
});

function activityStatusLabel(status: ActivityStatus): string {
  switch (status) {
    case 'running': return 'In progress';
    case 'completed': return 'Completed';
    case 'warning': return 'Needs attention';
    case 'failed': return 'Failed';
    case 'approval': return 'Approval needed';
  }
}

function activityStatusColor(status: ActivityStatus, palette: MobilePalette): string {
  switch (status) {
    case 'running': return palette.success;
    case 'completed': return palette.secondary;
    case 'warning':
    case 'approval': return palette.warning;
    case 'failed': return palette.danger;
  }
}

function activityIconName(icon: ProjectedActivityIcon, status: ActivityStatus): MilimIconName {
  if (status === 'failed') return 'x';
  if (status === 'warning' || status === 'approval') return 'info';
  if (status === 'completed') return 'check';
  switch (icon) {
    case 'file': return 'file';
    case 'image': return 'image';
    case 'worker': return 'bolt';
    case 'thinking': return 'sparkles';
    case 'command': return 'square';
    default: return 'bolt';
  }
}

function ActivityRow({row}: {row: ProjectedActivityRow}) {
  const {palette, styles} = useAppTheme();
  const color = activityStatusColor(row.status, palette);
  return (
    <View style={styles.activityRow}>
      <View style={styles.activityRowIcon}>
        <MilimIcon name={activityIconName(row.icon, row.status)} size={13} color={color} />
      </View>
      <View style={styles.activityRowBody}>
        <View style={styles.activityRowTopline}>
          <Text style={styles.activityRowLabel}>{row.label}</Text>
          <Text style={[styles.activityRowStatus, {color}]}>{activityStatusLabel(row.status)}</Text>
        </View>
        {row.detail ? (
          <ScrollView
            horizontal
            nestedScrollEnabled
            showsHorizontalScrollIndicator={false}
            contentContainerStyle={styles.activityDetailScroll}>
            <Text style={styles.activityRowDetail} selectable>{row.detail}</Text>
          </ScrollView>
        ) : null}
        {row.kind === 'change' && (row.additions !== undefined || row.deletions !== undefined) ? (
          <View style={styles.diffChips} accessibilityLabel={`${row.additions ?? 0} additions, ${row.deletions ?? 0} deletions`}>
            <Text style={[styles.diffChip, styles.diffChipAdded]}>+{row.additions ?? 0}</Text>
            <Text style={[styles.diffChip, styles.diffChipRemoved]}>-{row.deletions ?? 0}</Text>
          </View>
        ) : null}
      </View>
    </View>
  );
}

function RunDetailSection({label, value}: {label: string; value: unknown}) {
  const {palette, styles} = useAppTheme();
  const [open, setOpen] = useState(false);
  return (
    <View style={styles.runDetailSection}>
      <MotionPressable
        style={styles.runDetailSectionHeader}
        accessibilityLabel={`${open ? 'Collapse' : 'Expand'} ${label}`}
        onPress={() => setOpen(current => !current)}>
        <Text style={styles.runDetailSectionLabel}>{label}</Text>
        <MilimIcon name={open ? 'chevron-up' : 'chevron-down'} size={12} color={palette.muted} />
      </MotionPressable>
      {open ? (
        <ScrollView horizontal nestedScrollEnabled showsHorizontalScrollIndicator={false}>
          <Text style={styles.runDetailJson} selectable>{JSON.stringify(value, null, 2)}</Text>
        </ScrollView>
      ) : null}
    </View>
  );
}

function runEventGroups(events: RunEventV1[], inspection: RunInspectionV1) {
  const contains = (event: RunEventV1, needles: string[]) => needles.some(needle => event.type.includes(needle));
  const model = events.filter(event => contains(event, ['model_', 'request', 'response']));
  const tools = events.filter(event => contains(event, ['tool_', 'approval']));
  const inbox = events.filter(event => contains(event, ['inbox', 'steer', 'inject', 'followup']));
  const failures = events.filter(event => contains(event, ['error', 'fail', 'cancel', 'interrupt']));
  return [
    {label: 'Composition', value: inspection.composition},
    {label: 'Model steps', value: model},
    {label: 'Tools', value: tools},
    {label: 'Inbox', value: inbox},
    ...(inspection.run.error !== null || failures.length
      ? [{label: 'Failure information', value: {error: inspection.run.error, events: failures}}]
      : []),
  ];
}

function MobileRunDetails({
  runId,
  load,
  loadMore,
}: {
  runId: string;
  load: ReturnType<typeof useMilimController>['loadRunDetails'];
  loadMore: ReturnType<typeof useMilimController>['loadMoreRunEvents'];
}) {
  const {palette, styles} = useAppTheme();
  const [details, setDetails] = useState<{inspection: RunInspectionV1; events: RunEventPageV1} | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const requestDetails = async () => {
    if (details || loading) return;
    setLoading(true);
    setError(null);
    try {
      setDetails(await load(runId));
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setLoading(false);
    }
  };
  const requestMore = async () => {
    const next = details?.events.next_seq;
    if (!details || next == null || loading) return;
    setLoading(true);
    setError(null);
    try {
      const page = await loadMore(runId, next);
      setDetails(current => current ? {
        ...current,
        events: {...page, events: [...current.events.events, ...page.events]},
      } : current);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setLoading(false);
    }
  };

  if (!details) {
    return (
      <View style={styles.runDetailsFooter}>
        <MotionPressable
          style={styles.runDetailsAction}
          disabled={loading}
          onPress={() => void requestDetails()}>
          {loading ? <ActivityIndicator size="small" color={palette.muted} /> : <MilimIcon name="info" size={12} color={palette.muted} />}
          <Text style={styles.runDetailsActionText}>{loading ? 'Loading run details…' : 'Run details'}</Text>
        </MotionPressable>
        {error ? <Text style={styles.runDetailsError}>{error}</Text> : null}
      </View>
    );
  }

  return (
    <View style={styles.runDetailsPanel}>
      {runEventGroups(details.events.events, details.inspection).map(section => (
        <RunDetailSection key={section.label} label={section.label} value={section.value} />
      ))}
      {details.events.has_more ? (
        <MotionPressable style={styles.runDetailsAction} disabled={loading} onPress={() => void requestMore()}>
          {loading ? <ActivityIndicator size="small" color={palette.muted} /> : <MilimIcon name="refresh" size={12} color={palette.muted} />}
          <Text style={styles.runDetailsActionText}>{loading ? 'Loading…' : 'Load more events'}</Text>
        </MotionPressable>
      ) : null}
      {error ? <Text style={styles.runDetailsError}>{error}</Text> : null}
    </View>
  );
}

function ActivityGroup({
  group,
  runDetailsEnabled,
  loadRunDetails,
  loadMoreRunEvents,
}: {
  group: ProjectedActivityGroup;
  runDetailsEnabled: boolean;
  loadRunDetails: ReturnType<typeof useMilimController>['loadRunDetails'];
  loadMoreRunEvents: ReturnType<typeof useMilimController>['loadMoreRunEvents'];
}) {
  const {palette, styles} = useAppTheme();
  const [open, setOpen] = useState(group.status === 'running' || group.status === 'failed');
  const previousStatus = useRef(group.status);
  useEffect(() => {
    if (group.status === 'failed') setOpen(true);
    if (previousStatus.current === 'running' && group.status === 'completed') setOpen(false);
    previousStatus.current = group.status;
  }, [group.status]);
  const color = activityStatusColor(group.status, palette);
  const latest = group.rows.at(-1);
  return (
    <View style={[styles.activityGroup, group.status === 'failed' && styles.activityGroupFailed]}>
      <MotionPressable
        style={styles.activitySummary}
        accessibilityLabel={`${group.label}. ${activityStatusLabel(group.status)}. ${open ? 'Collapse' : 'Expand'} work details`}
        onPress={() => setOpen(current => !current)}>
        <View style={styles.activitySummaryIcon}>
          <MilimIcon name={activityIconName(latest?.icon ?? 'thinking', group.status)} size={14} color={color} />
        </View>
        <View style={styles.activitySummaryCopy}>
          <Text style={styles.activitySummaryLabel}>{group.label}</Text>
          {group.detail ? <Text style={styles.activitySummaryDetail}>{group.detail}</Text> : null}
        </View>
        <Text style={[styles.activitySummaryStatus, {color}]}>{activityStatusLabel(group.status)}</Text>
        <MilimIcon name={open ? 'chevron-up' : 'chevron-down'} size={13} color={palette.muted} />
      </MotionPressable>
      {open ? (
        <View style={styles.activityRows}>
          {group.rows.map(row => <ActivityRow key={row.id} row={row} />)}
          {runDetailsEnabled ? (
            <MobileRunDetails runId={group.runId} load={loadRunDetails} loadMore={loadMoreRunEvents} />
          ) : null}
        </View>
      ) : null}
    </View>
  );
}

const TranscriptItemView = React.memo(function MemoizedTranscriptItem({
  item,
  markdownStyles,
  execute,
  runDetailsEnabled,
  loadRunDetails,
  loadMoreRunEvents,
}: {
  item: ProjectedTranscriptItem;
  markdownStyles: ReturnType<typeof createMarkdownStyles>;
  execute: ReturnType<typeof useMilimController>['execute'];
  runDetailsEnabled: boolean;
  loadRunDetails: ReturnType<typeof useMilimController>['loadRunDetails'];
  loadMoreRunEvents: ReturnType<typeof useMilimController>['loadMoreRunEvents'];
}) {
  const {palette, styles} = useAppTheme();
  if (item.kind === 'model-change') {
    const previousModel = transcriptModelLabel(item.previousModel);
    const model = transcriptModelLabel(item.model);
    return (
      <View
        style={styles.modelChangeEvent}
        accessible
        accessibilityRole="text"
        accessibilityLabel={`Continuing with ${model}. Previously ${previousModel}. Thread retained.`}>
        <View style={styles.modelChangeLine} />
        <View style={styles.modelChangeCopy}>
          <View style={styles.modelChangeTopline}>
            <MilimIcon name="cube" size={14} color={palette.muted} />
            <Text style={styles.modelChangePrimary}>
              Continuing with <Text style={styles.modelChangeModel}>{model}</Text>
            </Text>
          </View>
          <Text style={styles.modelChangeDetail}>Previously {previousModel} · thread retained</Text>
        </View>
        <View style={styles.modelChangeLine} />
      </View>
    );
  }
  if (item.kind === 'activity') return (
    <ActivityGroup
      group={item}
      runDetailsEnabled={runDetailsEnabled}
      loadRunDetails={loadRunDetails}
      loadMoreRunEvents={loadMoreRunEvents}
    />
  );
  if (item.kind === 'approval') {
    if (item.approval) return <ApprovalCard approval={item.approval} execute={execute} inline />;
    const color = activityStatusColor(item.status, palette);
    return (
      <View style={styles.activityNotice}>
        <MilimIcon name={activityIconName('status', item.status)} size={14} color={color} />
        <View style={styles.activitySummaryCopy}>
          <Text style={styles.activitySummaryLabel}>{item.label}</Text>
          {item.detail ? <Text style={styles.activitySummaryDetail}>{item.detail}</Text> : null}
        </View>
        <Text style={[styles.activitySummaryStatus, {color}]}>{activityStatusLabel(item.status)}</Text>
      </View>
    );
  }
  return (
    <View style={[styles.message, item.role === 'user' ? styles.userMessage : styles.assistantMessage]}>
      {item.steering ? (
        <Text style={styles.messageRole}>{item.steeringPending ? 'STEER · PENDING' : 'STEER'}</Text>
      ) : null}
      {item.mailboxLabel ? <Text style={styles.messageRole}>{item.mailboxLabel}</Text> : null}
      {item.role === 'system' ? <Text style={styles.messageRole}>SYSTEM</Text> : null}
      {item.reasoning ? <ReasoningBlock text={item.reasoning} /> : null}
      <Markdown markdownit={mobileMarkdownParser} style={markdownStyles} rules={mobileMarkdownRules}>
        {item.content || '…'}
      </Markdown>
    </View>
  );
});

function PickerChip({
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

function ReasoningBlock({text}: {text: string}) {
  const {palette, styles} = useAppTheme();
  const [open, setOpen] = useState(false);
  return (
    <View style={styles.reasoningBlock}>
      <MotionPressable style={styles.reasoningHeader} hitSlop={8} onPress={() => setOpen(current => !current)}>
        <MilimIcon name={open ? 'chevron-up' : 'chevron-down'} size={12} color={palette.muted} />
        <Text style={styles.reasoningLabel}>{open ? 'Hide reasoning' : 'Show reasoning'}</Text>
      </MotionPressable>
      {open ? <Text style={styles.reasoning}>{text}</Text> : null}
    </View>
  );
}

function PickerSheetFrame({
  visible,
  title,
  subtitle,
  onClose,
  children,
  compact = false,
}: {
  visible: boolean;
  title: string;
  subtitle: string;
  onClose: () => void;
  children: React.ReactNode;
  compact?: boolean;
}) {
  const {palette, styles} = useAppTheme();
  const reduced = useReducedMotion();
  return (
    <Modal
      visible={visible}
      transparent
      statusBarTranslucent
      animationType={reduced ? 'none' : 'slide'}
      onRequestClose={onClose}>
      <View style={styles.pickerModal}>
        <SheetBackdropFade />
        <Pressable style={StyleSheet.absoluteFill} onPress={onClose} />
        <SafeAreaView style={[styles.pickerSheet, compact && styles.pickerSheetCompact]} edges={['bottom', 'left', 'right']}>
          <View style={styles.sheetHandle} />
          <View style={styles.pickerHeader}>
            <View style={styles.flex}>
              <Text style={styles.pickerTitle}>{title}</Text>
              <Text style={styles.pickerSubtitle}>{subtitle}</Text>
            </View>
            <MotionPressable style={styles.pickerClose} onPress={onClose} hitSlop={8}>
              <MilimIcon name="x" size={16} color={palette.secondary} />
            </MotionPressable>
          </View>
          {children}
        </SafeAreaView>
      </View>
    </Modal>
  );
}

const capabilityIcons: Record<MobileModelCapability, MilimIconName> = {
  vision: 'eye',
  tools: 'plug',
  reasoning: 'sparkles',
  fast: 'bolt',
  image: 'image',
  video: 'video',
  music: 'volume',
};

const reasoningEffortLabels: Record<string, string> = {
  auto: 'Auto',
  none: 'Off',
  minimal: 'Minimal',
  low: 'Low',
  medium: 'Medium',
  high: 'High',
  on: 'On',
  xhigh: 'X-high',
  max: 'Max',
};

function ModelPickerSheet({
  visible,
  hostId,
  models,
  favoriteIds,
  selectedId,
  reasoningEffortOverrides,
  onClose,
  onFavoriteIdsChange,
  onSelect,
}: {
  visible: boolean;
  hostId: string;
  models: JsonValue[];
  favoriteIds?: string[];
  selectedId: string | null;
  reasoningEffortOverrides?: Record<string, string>;
  onClose: () => void;
  onFavoriteIdsChange?: (favoriteModelIds: string[]) => Promise<void>;
  onSelect: (id: string, reasoningEffort?: string) => void;
}) {
  const {palette, styles} = useAppTheme();
  const [query, setQuery] = useState('');
  const [effortModelId, setEffortModelId] = useState<string | null>(null);
  const [preferences, setPreferences] = useState(DEFAULT_MODEL_PICKER_PREFERENCES);
  const [optimisticFavoriteIds, setOptimisticFavoriteIds] = useState<string[] | null>(null);
  const effectiveFavoriteIds = modelPickerFavoriteIds(
    optimisticFavoriteIds ?? favoriteIds,
    preferences.favorites,
  );
  const groups = useMemo(
    () => modelPickerGroups(models, query, effectiveFavoriteIds, preferences.favoritesOnly),
    [effectiveFavoriteIds, models, preferences.favoritesOnly, query],
  );
  const collapsedGroups = useMemo(
    () => new Set(preferences.collapsedGroups),
    [preferences.collapsedGroups],
  );
  const filtering = Boolean(query.trim()) || preferences.favoritesOnly;
  const rows = useMemo(
    () => groups.flatMap(group => {
      const collapsible = group.title !== 'Favorites' && !filtering;
      const collapsed = collapsible && collapsedGroups.has(group.title);
      return [
        {
          type: 'header' as const,
          key: `header:${group.title}`,
          title: group.title,
          count: group.models.length,
          brand: group.models[0]?.brand ?? null,
          collapsible,
          collapsed,
        },
        ...(collapsed ? [] : group.models.flatMap(model => [
          {type: 'model' as const, key: `${group.title}:${model.id}`, model},
          ...(effortModelId === model.id
            ? [{type: 'effort' as const, key: `${group.title}:${model.id}:effort`, model}]
            : []),
        ])),
      ];
    }),
    [collapsedGroups, effortModelId, filtering, groups],
  );
  useEffect(() => {
    if (!visible) {
      setQuery('');
      setEffortModelId(null);
      return;
    }
    setPreferences(DEFAULT_MODEL_PICKER_PREFERENCES);
    setOptimisticFavoriteIds(null);
    if (!hostId) return;
    let cancelled = false;
    void readModelPickerPreferences(hostId)
      .then(next => {
        if (!cancelled) setPreferences(next);
      })
      .catch(showError);
    return () => {
      cancelled = true;
    };
  }, [hostId, visible]);
  useEffect(() => {
    setOptimisticFavoriteIds(null);
  }, [favoriteIds]);
  const updatePreferences = useCallback((next: typeof preferences) => {
    setPreferences(next);
    if (hostId) void saveModelPickerPreferences(hostId, next).catch(showError);
  }, [hostId]);
  const toggleFavorite = useCallback((favoriteModelId: string) => {
    const next = toggledModelFavoriteIds(effectiveFavoriteIds, favoriteModelId);
    if (favoriteIds !== undefined && onFavoriteIdsChange) {
      setOptimisticFavoriteIds(next);
      void onFavoriteIdsChange(next).catch(error => {
        setOptimisticFavoriteIds(null);
        showError(error);
      });
      return;
    }
    updatePreferences({...preferences, favorites: next});
  }, [effectiveFavoriteIds, favoriteIds, onFavoriteIdsChange, preferences, updatePreferences]);
  const toggleGroup = useCallback((title: string) => {
    const next = new Set(preferences.collapsedGroups);
    if (next.has(title)) next.delete(title);
    else next.add(title);
    updatePreferences({...preferences, collapsedGroups: [...next]});
  }, [preferences, updatePreferences]);
  return (
    <PickerSheetFrame
      visible={visible}
      title="Choose model"
      subtitle={`${models.length} available from your desktop`}
      onClose={onClose}>
      <View style={styles.pickerSearch}>
        <MilimIcon name="search" size={15} color={palette.muted} />
        <TextInput
          style={styles.pickerSearchInput}
          value={query}
          onChangeText={setQuery}
          placeholder="Search models or providers"
          placeholderTextColor={palette.placeholder}
          autoCapitalize="none"
          autoCorrect={false}
        />
        {query ? (
          <MotionPressable style={styles.pickerSearchClear} onPress={() => setQuery('')} hitSlop={8}>
            <MilimIcon name="x" size={13} color={palette.muted} />
          </MotionPressable>
        ) : null}
      </View>
      <FlatList
        style={styles.pickerListView}
        contentContainerStyle={styles.pickerList}
        data={rows}
        keyExtractor={item => item.key}
        keyboardShouldPersistTaps="handled"
        initialNumToRender={10}
        windowSize={7}
        ListEmptyComponent={<Empty title="No matching models" copy="Try a model name, provider, or runtime." />}
        renderItem={({item}) => item.type === 'header' ? (
          <MotionPressable
            style={styles.pickerGroupHeader}
            disabled={!item.collapsible}
            accessibilityRole={item.collapsible ? 'button' : undefined}
            accessibilityState={item.collapsible ? {expanded: !item.collapsed} : undefined}
            onPress={() => item.collapsible && toggleGroup(item.title)}>
            {item.title === 'Favorites'
              ? <MilimIcon name="star" filled size={13} color={palette.accent} />
              : <ProviderIcon brand={item.brand} size={13} color={palette.secondary} />}
            <Text style={styles.pickerGroupTitle}>{item.title.toUpperCase()}</Text>
            <Text style={styles.pickerGroupCount}>{item.count}</Text>
            {item.collapsible
              ? <MilimIcon name={item.collapsed ? 'chevron-right' : 'chevron-down'} size={12} color={palette.muted} />
              : null}
          </MotionPressable>
        ) : item.type === 'effort' ? (
          <ReasoningEffortChoices
            model={item.model}
            selected={reasoningEffortOverrides?.[item.model.id] ?? 'auto'}
            onSelect={effort => onSelect(item.model.id, effort)}
          />
        ) : (
          <ModelPickerRow
            model={item.model}
            selected={item.model.id === selectedId}
            favorite={effectiveFavoriteIds.includes(item.model.id)}
            reasoningEffort={reasoningEffortOverrides?.[item.model.id] ?? 'auto'}
            onPress={() => onSelect(item.model.id)}
            onFavorite={() => toggleFavorite(item.model.id)}
            onToggleReasoning={item.model.reasoningEfforts.length
              ? () => setEffortModelId(current => current === item.model.id ? null : item.model.id)
              : undefined}
          />
        )}
      />
      <MotionPressable
        style={styles.pickerFavoritesOnly}
        accessibilityRole="switch"
        accessibilityState={{checked: preferences.favoritesOnly}}
        onPress={() => updatePreferences({...preferences, favoritesOnly: !preferences.favoritesOnly})}>
        <MilimIcon name="star" filled={preferences.favoritesOnly} size={14} color={preferences.favoritesOnly ? palette.accent : palette.muted} />
        <Text style={styles.pickerFavoritesOnlyText}>Favorites only</Text>
        <View style={[styles.pickerSwitch, preferences.favoritesOnly && styles.pickerSwitchOn]}>
          <View style={[styles.pickerSwitchThumb, preferences.favoritesOnly && styles.pickerSwitchThumbOn]} />
        </View>
      </MotionPressable>
    </PickerSheetFrame>
  );
}

function ModelPickerRow({
  model,
  selected,
  favorite,
  reasoningEffort,
  onPress,
  onFavorite,
  onToggleReasoning,
}: {
  model: MobileModelOption;
  selected: boolean;
  favorite: boolean;
  reasoningEffort: string;
  onPress: () => void;
  onFavorite: () => void;
  onToggleReasoning?: () => void;
}) {
  const {palette, styles} = useAppTheme();
  const visibleCapabilities = onToggleReasoning
    ? model.capabilities.filter(capability => capability !== 'reasoning')
    : model.capabilities;
  return (
    <View style={[styles.pickerRow, selected && styles.pickerRowSelected]}>
      <MotionPressable style={styles.pickerRowMain} onPress={onPress}>
        <View style={styles.pickerRowIcon}>
          <ProviderIcon brand={model.brand} size={17} color={selected ? palette.accent : palette.secondary} />
        </View>
        <View style={styles.pickerRowBody}>
          <View style={styles.pickerRowTopline}>
            <Text style={styles.pickerRowTitle} numberOfLines={1}>{model.label}</Text>
            {selected ? <MilimIcon name="check" size={14} color={palette.accent} /> : null}
          </View>
          <View style={styles.pickerRowMeta}>
            <Text style={styles.pickerRowRoute} numberOfLines={1}>
              {[model.route, model.detail].filter(Boolean).join(' · ')}
            </Text>
            {visibleCapabilities.length ? (
              <View style={styles.capabilityRow}>
                {visibleCapabilities.slice(0, 5).map(capability => (
                  <View
                    key={capability}
                    style={[styles.pickerCapabilityIcon, capability === 'fast' && styles.pickerCapabilityFastIcon]}
                    accessibilityLabel={capability}>
                    <MilimIcon name={capabilityIcons[capability]} size={12} color={palette.muted} />
                  </View>
                ))}
              </View>
            ) : null}
          </View>
        </View>
      </MotionPressable>
      {onToggleReasoning ? (
        <MotionPressable
          style={styles.pickerRowEffort}
          hitSlop={4}
          accessibilityLabel={`Reasoning effort for ${model.label}: ${reasoningEffortLabels[reasoningEffort] ?? reasoningEffort}`}
          onPress={onToggleReasoning}>
          <MilimIcon name="sparkles" size={12} color={reasoningEffort === 'auto' ? palette.muted : palette.accent} />
          {reasoningEffort !== 'auto'
            ? <Text style={styles.pickerRowEffortText}>{reasoningEffortLabels[reasoningEffort] ?? reasoningEffort}</Text>
            : null}
        </MotionPressable>
      ) : null}
      <MotionPressable
        style={styles.pickerRowFavorite}
        hitSlop={4}
        accessibilityLabel={favorite ? `Remove ${model.label} from favorites` : `Add ${model.label} to favorites`}
        onPress={onFavorite}>
        <MilimIcon name="star" filled={favorite} size={14} color={favorite ? palette.accent : palette.muted} />
      </MotionPressable>
    </View>
  );
}

function ReasoningEffortChoices({
  model,
  selected,
  onSelect,
}: {
  model: MobileModelOption;
  selected: string;
  onSelect: (effort: string) => void;
}) {
  const {styles} = useAppTheme();
  const choices = ['auto', ...new Set(model.reasoningEfforts)];
  return (
    <View style={styles.reasoningEffortChoices}>
      <Text style={styles.reasoningEffortLabel}>Reasoning</Text>
      <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={styles.reasoningEffortScroll}>
        {choices.map(effort => (
          <MotionPressable
            key={effort}
            style={[styles.reasoningEffortChoice, effort === selected && styles.reasoningEffortChoiceSelected]}
            onPress={() => onSelect(effort)}>
            <Text style={[styles.reasoningEffortChoiceText, effort === selected && styles.reasoningEffortChoiceTextSelected]}>
              {reasoningEffortLabels[effort] ?? effort}
            </Text>
          </MotionPressable>
        ))}
      </ScrollView>
    </View>
  );
}

function AgentPickerSheet({
  visible,
  agents,
  selectedId,
  onClose,
  onSelect,
}: {
  visible: boolean;
  agents: NonNullable<ReturnType<typeof useMilimController>['bootstrap']>['agents'];
  selectedId: string | null;
  onClose: () => void;
  onSelect: (id: string) => void;
}) {
  const {palette, styles} = useAppTheme();
  const [query, setQuery] = useState('');
  const filtered = useMemo(() => agents.filter(agent =>
    !query.trim() || [agent.name, agent.description, agent.id].some(value =>
      value.toLowerCase().includes(query.trim().toLowerCase()),
    ),
  ), [agents, query]);
  useEffect(() => {
    if (!visible) setQuery('');
  }, [visible]);
  return (
    <PickerSheetFrame
      visible={visible}
      title="Choose Agent"
      subtitle="Use the same Agent definitions as milim desktop"
      onClose={onClose}>
      <View style={styles.pickerSearch}>
        <MilimIcon name="search" size={15} color={palette.muted} />
        <TextInput
          style={styles.pickerSearchInput}
          value={query}
          onChangeText={setQuery}
          placeholder="Search Agents"
          placeholderTextColor={palette.placeholder}
        />
      </View>
      <FlatList
        data={filtered}
        keyExtractor={agent => agent.id}
        contentContainerStyle={styles.pickerList}
        keyboardShouldPersistTaps="handled"
        initialNumToRender={12}
        windowSize={7}
        ListHeaderComponent={!query ? (
          <MotionPressable style={[styles.pickerRow, !selectedId && styles.pickerRowSelected]} onPress={() => onSelect('')}>
            <View style={styles.pickerRowIcon}>
              <MilimIcon name="x" size={14} color={palette.secondary} />
            </View>
            <View style={styles.pickerRowBody}>
              <Text style={styles.pickerRowTitle}>No Agent</Text>
              <Text style={styles.pickerRowRoute}>Use the thread’s regular model and tools</Text>
            </View>
            {!selectedId ? <MilimIcon name="check" size={15} color={palette.accent} /> : null}
          </MotionPressable>
        ) : null}
        ListEmptyComponent={<Empty title="No matching Agents" copy="Agent names and descriptions are searchable here." />}
        renderItem={({item: agent}) => (
          <MotionPressable
            style={[styles.pickerRow, agent.id === selectedId && styles.pickerRowSelected]}
            onPress={() => onSelect(agent.id)}>
            <View style={[styles.pickerRowIcon, agent.id === selectedId && styles.pickerRowIconSelected]}>
              <AgentAvatar {...agent} size={24} />
            </View>
            <View style={styles.pickerRowBody}>
              <View style={styles.pickerRowTopline}>
                <Text style={styles.pickerRowTitle} numberOfLines={1}>{agent.name}</Text>
                {agent.id === selectedId ? <MilimIcon name="check" size={15} color={palette.accent} /> : null}
              </View>
              <Text style={styles.pickerRowDescription} numberOfLines={2}>{agent.description || 'Custom milim Agent'}</Text>
              <Text style={styles.pickerRowRoute}>
                {agent.enabled_tool_count} tools · {agent.enabled_skill_count} skills
              </Text>
            </View>
          </MotionPressable>
        )}
      />
    </PickerSheetFrame>
  );
}

function AttentionScreen({
  approvals,
  queuedTurns,
  timeline,
  execute,
  onBack,
}: {
  approvals: PendingApprovalV1[];
  queuedTurns: QueuedTurnV1[];
  timeline: NonNullable<ReturnType<typeof useMilimController>['timeline']>['items'];
  execute: ReturnType<typeof useMilimController>['execute'];
  onBack: () => void;
}) {
  const {styles} = useAppTheme();
  const proposals = timeline.filter(item => item.type.includes('worker') && JSON.stringify(item.data).includes('proposed'));
  return (
    <ScrollView style={styles.screen} contentContainerStyle={styles.list}>
      <View style={styles.pageHeadingRow}>
        <IconButton icon="arrow-left" label="Back to chat" onPress={onBack} />
        <View style={styles.pageHeadingCopy}>
          <Text style={styles.eyebrow}>REVIEW</Text>
          <Text style={styles.screenTitle}>Attention</Text>
        </View>
      </View>
      {!approvals.length && !queuedTurns.length && !proposals.length ? <Empty title="Nothing pending" copy="Review approvals, paused queued turns, and Worker proposals appear here when you foreground the app." /> : null}
      {approvals.map(approval => <ApprovalCard key={approval.id} approval={approval} execute={execute} />)}
      {queuedTurns.map(turn => (
        <View key={turn.id} style={styles.attentionCard}>
          <Text style={styles.messageRole}>QUEUED TURN</Text>
          <Text style={styles.attentionTitle}>Waiting in this thread</Text>
          <Text style={styles.help}>A stopped run leaves queued work paused until you resume or delete it.</Text>
          <View style={styles.actionRow}>
            <Button label="Delete" icon="trash" tone="danger" onPress={() => void execute({command_id: newCommandId(), kind: 'turn.queue_delete', thread_id: turn.thread_id, payload: {queue_id: turn.id}}).catch(showError)} />
            <Button label="Resume" icon="arrow-up" onPress={() => void execute({command_id: newCommandId(), kind: 'turn.queue_resume', thread_id: turn.thread_id, payload: {queue_id: turn.id}}).catch(showError)} />
          </View>
        </View>
      ))}
      {proposals.map(proposal => {
        const data = proposal.data as Record<string, unknown>;
        const run = (data.run ?? data) as Record<string, unknown>;
        const runId = typeof run.id === 'string' ? run.id : proposal.run_id;
        return (
          <View key={proposal.id} style={styles.attentionCard}>
            <Text style={styles.messageRole}>WORKER PROPOSAL</Text>
            <Text style={styles.attentionTitle}>{String(run.title ?? 'Delegated work')}</Text>
            <Text style={styles.help}>The proposal uses frozen Agent snapshots. Later Agent edits cannot change it.</Text>
            <View style={styles.actionRow}>
              <Button label="Continue solo" icon="sparkles" tone="quiet" disabled={!runId} onPress={() => runId && void execute({command_id: newCommandId(), kind: 'worker.continue_solo', thread_id: proposal.thread_id, payload: {run_id: runId}}).catch(showError)} />
              <Button label="Start workers" icon="bolt" disabled={!runId} onPress={() => runId && void execute({command_id: newCommandId(), kind: 'worker.start', thread_id: proposal.thread_id, payload: {run_id: runId}}).catch(showError)} />
            </View>
          </View>
        );
      })}
    </ScrollView>
  );
}

function ApprovalCard({approval, execute, inline = false}: {approval: PendingApprovalV1; execute: ReturnType<typeof useMilimController>['execute']; inline?: boolean}) {
  const {palette, styles} = useAppTheme();
  const [response, setResponse] = useState<Record<string, string>>({});
  const request = approval.request as Record<string, unknown>;
  const supported = ['command', 'file_change', 'permission_elevation', 'mcp_form', 'mcp_url'].includes(approval.kind);
  const properties = ((request.schema as Record<string, unknown> | undefined)?.properties ?? {}) as Record<string, unknown>;
  const descriptors = Array.isArray(request.fields)
    ? (request.fields as Array<Record<string, unknown>>)
    : Object.keys(properties).map(name => ({name, label: name}));
  const fields = approval.kind === 'mcp_form'
    ? descriptors
        .map(field => typeof field.name === 'string' ? field.name : '')
        .filter(Boolean)
        .slice(0, 8)
    : [];
  const resolve = (decision: 'approve' | 'deny') => execute({
    command_id: newCommandId(),
    kind: 'approval.resolve',
    thread_id: approval.thread_id,
    payload: {
      approval_id: approval.id,
      decision,
      ...(decision === 'approve' && fields.length ? {response} : {}),
    },
  });
  const requestDetail = String(
    request.command ??
    request.arguments ??
    request.prompt ??
    request.message ??
    request.reason ??
    request.url ??
    'Review this request before continuing.',
  );
  return (
    <View style={[styles.attentionCard, inline && styles.inlineApprovalCard]}>
      <Text style={styles.messageRole}>APPROVAL · {approval.kind.replaceAll('_', ' ').toUpperCase()}</Text>
      <Text style={styles.attentionTitle}>{String(request.name ?? request.title ?? request.server_name ?? 'Runtime approval')}</Text>
      <ScrollView horizontal nestedScrollEnabled showsHorizontalScrollIndicator contentContainerStyle={styles.approvalDetailScroll}>
        <Text style={styles.codeBlock} selectable>{requestDetail}</Text>
      </ScrollView>
      {fields.map(field => (
        <TextInput
          key={field}
          style={styles.input}
          value={response[field] ?? ''}
          onChangeText={value => setResponse(current => ({...current, [field]: value}))}
          placeholder={String(descriptors.find(descriptor => descriptor.name === field)?.label ?? field)}
          placeholderTextColor={palette.placeholder}
          autoCapitalize="none"
        />
      ))}
      {!supported ? <Text style={styles.missing}>This schema is not supported on mobile and can only be denied.</Text> : null}
      <View style={styles.actionRow}>
        <Button label="Deny" icon="x" tone="danger" onPress={() => void resolve('deny').catch(showError)} />
        <Button label="Approve" icon="check" disabled={!supported} onPress={() => void resolve('approve').catch(showError)} />
      </View>
    </View>
  );
}

const HostsScreen = React.memo(function MemoizedHostsScreen({
  controller,
  onPair,
  onBack,
}: {
  controller: HostsController;
  onPair: () => void;
  onBack: () => void;
}) {
  const {palette, styles} = useAppTheme();
  const [manual, setManual] = useState('');
  const [manualOpen, setManualOpen] = useState(false);
  const [detailsHostId, setDetailsHostId] = useState<string | null>(null);
  const activeHost = controller.activeHost;
  const otherHosts = controller.hosts.filter(host => host.hostId !== activeHost?.hostId);
  const detailsHost = controller.hosts.find(host => host.hostId === detailsHostId) ?? null;
  const compatible = Boolean(
    activeHost && activeHost.protocol.min <= 1 && activeHost.protocol.max >= 1,
  );
  return (
    <View style={styles.screen}>
      <ScrollView contentContainerStyle={styles.hostList}>
        <View style={styles.pageHeadingRow}>
          <IconButton icon="arrow-left" label="Back to chat" onPress={onBack} />
          <View style={styles.pageHeadingCopy}>
            <Text style={styles.eyebrow}>DIRECT CONNECTIONS</Text>
            <Text style={styles.screenTitle}>Desktop hosts</Text>
          </View>
          <MotionPressable style={styles.hostPairAction} onPress={onPair} accessibilityLabel="Pair another desktop">
            <MilimIcon name="plus" size={14} color={palette.secondary} />
            <Text style={styles.hostPairActionText}>Pair</Text>
          </MotionPressable>
        </View>

        {activeHost ? (
          <View style={styles.hostPrimaryCard}>
            <View style={styles.hostPrimaryTopline}>
              <Image source={milimLogo} style={styles.hostMark} />
              <View style={styles.hostPrimaryBody}>
                <Text style={styles.hostName} numberOfLines={1}>{lowercaseMilimBrand(activeHost.displayName)}</Text>
                <Text style={styles.hostEndpoint} numberOfLines={1}>{friendlyEndpoint(activeHost.lastSuccessfulUrl)}</Text>
              </View>
              <View style={[styles.hostStatusPill, controller.status === 'online' && styles.hostStatusPillOnline]}>
                <View style={[styles.dot, controller.status === 'online' && styles.dotOnline]} />
                <Text style={styles.hostStatusText}>{controller.status}</Text>
              </View>
              <MotionPressable
                style={styles.hostMenu}
                onPress={() => setDetailsHostId(activeHost.hostId)}
                accessibilityLabel={`Details for ${lowercaseMilimBrand(activeHost.displayName)}`}>
                <MilimIcon name="more-horizontal" size={17} color={palette.muted} />
              </MotionPressable>
            </View>
            <View style={styles.hostPrimaryFooter}>
              <Text style={styles.hostActivity}>{relativeConnectionTime(activeHost.lastConnectedAt)}</Text>
              <View style={styles.hostCompatibility}>
                <MilimIcon name={compatible ? 'check' : 'x'} size={12} color={compatible ? palette.success : palette.danger} />
                <Text style={styles.hostCompatibilityText}>Protocol v1 {compatible ? 'compatible' : 'unsupported'}</Text>
              </View>
            </View>
          </View>
        ) : null}

        {otherHosts.length ? (
          <View style={styles.hostSection}>
            <Text style={styles.hostSectionLabel}>SAVED DESKTOPS</Text>
            <View style={styles.hostRows}>
              {otherHosts.map(host => (
                <Pressable key={host.hostId} style={styles.hostRow} onPress={() => controller.setActiveHost(host.hostId)}>
                  <Image source={milimLogo} style={styles.hostRowMark} />
                  <View style={styles.hostRowBody}>
                    <Text style={styles.hostRowTitle} numberOfLines={1}>{lowercaseMilimBrand(host.displayName)}</Text>
                    <Text style={styles.hostRowMeta} numberOfLines={1}>{relativeConnectionTime(host.lastConnectedAt)}</Text>
                  </View>
                  <Text style={styles.hostSwitchText}>Switch</Text>
                </Pressable>
              ))}
            </View>
          </View>
        ) : null}

        <View style={styles.hostSection}>
          <MotionPressable
            style={styles.hostDisclosure}
            onPress={() => setManualOpen(value => !value)}
            accessibilityLabel={`${manualOpen ? 'Close' : 'Open'} manual connection form`}>
            <View style={styles.hostDisclosureIcon}>
              <MilimIcon name="link" size={15} color={palette.secondary} />
            </View>
            <View style={styles.hostDisclosureBody}>
              <Text style={styles.hostDisclosureTitle}>Connect manually</Text>
              <Text style={styles.hostDisclosureCopy}>Add another address for this desktop</Text>
            </View>
            <MilimIcon name={manualOpen ? 'chevron-up' : 'chevron-down'} size={14} color={palette.muted} />
          </MotionPressable>
          {manualOpen ? (
            <View style={styles.hostManualForm}>
              <Text style={styles.help}>Tailscale HTTPS is recommended. Use plain HTTP only on a trusted LAN.</Text>
              <TextInput
                style={styles.input}
                value={manual}
                onChangeText={setManual}
                placeholder="https://desktop.tailnet.ts.net:10000"
                placeholderTextColor={palette.placeholder}
                autoCapitalize="none"
                autoCorrect={false}
                keyboardType="url"
              />
              <View style={styles.hostManualActions}>
                <Button
                  label="Save address"
                  icon="check"
                  tone="quiet"
                  disabled={!manual.trim()}
                  onPress={() => void controller.addManualHostCandidate(manual).then(() => {
                    setManual('');
                    setManualOpen(false);
                  }).catch(showError)}
                />
              </View>
            </View>
          ) : null}
        </View>

        <View style={styles.hostInfoRow}>
          <View style={styles.hostDisclosureIcon}>
            <MilimIcon name="info" size={15} color={palette.secondary} />
          </View>
          <View style={styles.hostDisclosureBody}>
            <Text style={styles.hostDisclosureTitle}>Foreground catch-up</Text>
            <Text style={styles.hostDisclosureCopy}>milim refreshes when reopened; background push is not enabled in v1.</Text>
          </View>
        </View>
      </ScrollView>

      <PickerSheetFrame
        visible={Boolean(detailsHost)}
        title={detailsHost ? lowercaseMilimBrand(detailsHost.displayName) : 'Desktop details'}
        subtitle="Paired desktop"
        compact
        onClose={() => setDetailsHostId(null)}>
        <View style={styles.hostDetails}>
          <View style={styles.hostDetailBlock}>
            <Text style={styles.drawerProjectLabel}>CONNECTION ADDRESS</Text>
            <Text style={styles.hostDetailValue} selectable>{detailsHost?.lastSuccessfulUrl ?? 'No reachable endpoint'}</Text>
          </View>
          <View style={styles.hostDetailGrid}>
            <View style={styles.hostDetailStat}>
              <Text style={styles.drawerProjectLabel}>PROTOCOL</Text>
              <Text style={styles.hostDetailStatValue}>{detailsHost ? `${detailsHost.protocol.min}–${detailsHost.protocol.max}` : '—'}</Text>
            </View>
            <View style={styles.hostDetailStat}>
              <Text style={styles.drawerProjectLabel}>LAST CONNECTION</Text>
              <Text style={styles.hostDetailStatValue}>{detailsHost?.lastConnectedAt ? new Date(detailsHost.lastConnectedAt).toLocaleString() : 'Never'}</Text>
            </View>
          </View>
          {detailsHost && detailsHost.hostId !== activeHost?.hostId ? (
            <ActionSheetButton icon="check" label="Use this desktop" onPress={() => {
              controller.setActiveHost(detailsHost.hostId);
              setDetailsHostId(null);
            }} />
          ) : null}
          {detailsHost?.hostId === activeHost?.hostId ? (
            <ActionSheetButton icon="trash" label="Revoke this phone" danger onPress={() => Alert.alert(
              'Remove host?',
              'This removes the local cache and asks the desktop to revoke this phone.',
              [
                {text: 'Cancel', style: 'cancel'},
                {text: 'Remove', style: 'destructive', onPress: () => {
                  setDetailsHostId(null);
                  void controller.removeHost().catch(showError);
                }},
              ],
            )} />
          ) : null}
        </View>
      </PickerSheetFrame>
    </View>
  );
});

function Button({
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

function IconButton({
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

function ActionSheetButton({icon, label, onPress, danger}: {icon: MilimIconName; label: string; onPress: () => void; danger?: boolean}) {
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

function Empty({title, copy, action, onAction}: {title: string; copy: string; action?: string; onAction?: () => void}) {
  const {styles} = useAppTheme();
  return (
    <View style={styles.empty}>
      <Text style={styles.emptyTitle}>{title}</Text>
      <Text style={[styles.help, styles.emptyCopy]}>{copy}</Text>
      {action && onAction ? <Button label={action} tone="quiet" onPress={onAction} /> : null}
    </View>
  );
}

function modelId(value: JsonValue): string | null {
  if (typeof value === 'string') return value;
  if (typeof value === 'object' && value !== null && !Array.isArray(value)) {
    const record = value as Record<string, JsonValue | undefined>;
    if (typeof record.id === 'string') return record.id;
  }
  return null;
}

async function confirmDestructive(execute: ReturnType<typeof useMilimController>['execute'], command: ControlCommandV1) {
  try {
    const challenge = await execute(command);
    if (challenge.status !== 'needs_confirmation' || !challenge.confirmation_token) return;
    Alert.alert('Confirm destructive action', challenge.message ?? 'This cannot be undone.', [
      {text: 'Cancel', style: 'cancel'},
      {
        text: 'Confirm',
        style: 'destructive',
        onPress: () => void execute({...command, confirmation_token: challenge.confirmation_token}).catch(showError),
      },
    ]);
  } catch (error) {
    showError(error);
  }
}

function showError(error: unknown) {
  Alert.alert('milim', error instanceof Error ? error.message : String(error));
}

function createMarkdownStyles(theme: MobileTheme) {
  const {fontFamily, monoFamily, palette} = theme;
  return {
    body: {color: palette.text, fontFamily, fontSize: 14.5, lineHeight: 22},
    code_inline: {color: palette.accent, backgroundColor: palette.raised, fontFamily: monoFamily},
    code_block: {color: palette.text, backgroundColor: palette.bg, borderColor: palette.border, fontFamily: monoFamily},
    fence: {color: palette.text, backgroundColor: palette.bg, borderColor: palette.border, fontFamily: monoFamily},
    link: {color: palette.accent},
  };
}

function createStyles(theme: MobileTheme) {
  const {cardRadius, fontFamily, inputRadius, monoFamily, palette} = theme;
  return StyleSheet.create({
  appText: fontFamily ? {fontFamily} : {},
  root: {flex: 1, backgroundColor: palette.bg},
  app: {flex: 1, backgroundColor: 'transparent'},
  backgroundImage: {position: 'absolute', top: 0, right: 0, bottom: 0, left: 0},
  backgroundImageCover: {transform: [{scale: 1.06}]},
  backgroundDim: {backgroundColor: 'rgba(0,0,0,0.18)'},
  topbar: {height: 46, paddingHorizontal: 12, flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', backgroundColor: palette.sidebar, borderBottomWidth: StyleSheet.hairlineWidth, borderBottomColor: palette.border},
  topbarLeading: {flex: 1, minWidth: 0, flexDirection: 'row', alignItems: 'center', gap: 5},
  brandGroup: {minHeight: 38, flexShrink: 1, flexDirection: 'row', alignItems: 'center', gap: 8, marginLeft: -5, paddingHorizontal: 5, borderRadius: 8},
  brandMark: {width: 23, height: 23, resizeMode: 'contain'},
  brand: {color: palette.text, fontSize: 13.5, lineHeight: 15, fontWeight: '700', letterSpacing: -0.35},
  hostLabel: {color: palette.muted, fontSize: 9, lineHeight: 11, maxWidth: 210},
  connection: {height: 25, flexDirection: 'row', alignItems: 'center', gap: 5, paddingHorizontal: 8, borderRadius: 8, borderWidth: 1, borderColor: palette.border, backgroundColor: palette.panel},
  connectionOnline: {borderColor: palette.borderStrong},
  connectionCount: {minWidth: 16, height: 16, marginRight: -4, paddingHorizontal: 4, borderRadius: 8, overflow: 'hidden', backgroundColor: palette.text, color: palette.accentInk, fontSize: 8.5, lineHeight: 16, fontWeight: '800', textAlign: 'center'},
  topbarDestinationActive: {backgroundColor: palette.raised},
  dot: {width: 6, height: 6, borderRadius: 3, backgroundColor: palette.muted},
  dotOnline: {backgroundColor: palette.success},
  connectionText: {color: palette.secondary, fontSize: 9, fontWeight: '700', textTransform: 'uppercase', letterSpacing: 0.7},
  topbarActions: {flexDirection: 'row', alignItems: 'center', gap: 5},
  topbarButton: {width: 34, height: 34, borderRadius: 8, alignItems: 'center', justifyContent: 'center', borderWidth: 1, borderColor: palette.border, backgroundColor: palette.panel},
  errorBanner: {paddingHorizontal: 16, paddingVertical: 9, backgroundColor: palette.dangerSurface, borderBottomWidth: 1, borderBottomColor: palette.dangerBorder, flexDirection: 'row', gap: 12, alignItems: 'center'},
  errorText: {color: palette.danger, fontSize: 12, flex: 1},
  retry: {color: palette.text, fontSize: 11, fontWeight: '700'},
  content: {flex: 1},
  screen: {flex: 1, paddingHorizontal: 12, paddingTop: 8},
  screenHeader: {height: 38, flexDirection: 'row', alignItems: 'baseline', gap: 8},
  screenSubtitle: {color: palette.muted, fontSize: 10.5},
  eyebrow: {color: palette.secondary, fontSize: 9.5, fontWeight: '700', letterSpacing: 1.5},
  screenTitle: {color: palette.text, fontSize: 20, lineHeight: 25, fontWeight: '700', letterSpacing: -0.55},
  input: {minHeight: 40, paddingHorizontal: 11, paddingVertical: 8, borderRadius: inputRadius, borderWidth: 1, borderColor: palette.borderStrong, backgroundColor: palette.input, color: palette.text, fontFamily, fontSize: 13.5},
  flex: {flex: 1},
  list: {flexGrow: 1, paddingBottom: 24, gap: 2},
  threadCard: {minHeight: 46, flexDirection: 'row', alignItems: 'center', backgroundColor: 'transparent', borderWidth: 1, borderColor: 'transparent', borderRadius: 8, paddingLeft: 10, paddingRight: 4, gap: 5},
  threadCardSelected: {backgroundColor: palette.panel, borderColor: palette.border},
  threadTopline: {flex: 1, minWidth: 0, flexDirection: 'row', alignItems: 'center', gap: 8},
  threadTitle: {color: palette.text, fontSize: 13.5, fontWeight: '600', flex: 1},
  threadOrigin: {color: palette.muted, fontSize: 9, fontWeight: '700', letterSpacing: 0.4, textTransform: 'uppercase'},
  queued: {color: palette.warning, fontSize: 9.5, fontWeight: '800'},
  threadMenu: {width: 38, height: 38, borderRadius: 7, alignItems: 'center', justifyContent: 'center'},
  drawerBackdrop: {flex: 1, flexDirection: 'row-reverse'},
  drawerDismiss: {flex: 1},
  drawer: {height: '100%', borderRightWidth: 1, borderRightColor: palette.glassEdge, backgroundColor: palette.popover},
  drawerEdgeFade: {position: 'absolute', left: '100%', top: 0, bottom: 0, width: 96},
  drawerSafe: {flex: 1},
  drawerHeader: {minHeight: 72, flexDirection: 'row', alignItems: 'center', gap: 12, paddingHorizontal: 14, paddingTop: 5, borderBottomWidth: StyleSheet.hairlineWidth, borderBottomColor: palette.border},
  drawerHeading: {flex: 1, gap: 2},
  drawerTitleRow: {flexDirection: 'row', alignItems: 'center', gap: 7},
  drawerTitle: {color: palette.text, fontSize: 20, lineHeight: 24, fontWeight: '700', letterSpacing: -0.55},
  drawerCount: {minWidth: 21, height: 20, paddingHorizontal: 6, borderRadius: 7, overflow: 'hidden', backgroundColor: palette.raised, color: palette.secondary, fontSize: 10, lineHeight: 20, fontWeight: '800', textAlign: 'center'},
  drawerClose: {width: 38, height: 38, borderRadius: 8, alignItems: 'center', justifyContent: 'center', borderWidth: 1, borderColor: palette.border, backgroundColor: palette.panel},
  drawerCreateRow: {flexDirection: 'row', gap: 7, paddingHorizontal: 12, paddingTop: 11, paddingBottom: 8},
  drawerCreateButton: {width: 40, height: 40, borderRadius: 8, alignItems: 'center', justifyContent: 'center', backgroundColor: palette.accent},
  drawerGroups: {flex: 1},
  drawerGroupList: {paddingHorizontal: 8, paddingTop: 2, paddingBottom: 20},
  drawerGroupSeparator: {height: 8},
  drawerGroup: {gap: 2},
  drawerGroupHeader: {minHeight: 48, flexDirection: 'row', alignItems: 'center'},
  drawerGroupToggle: {flex: 1, minHeight: 48, flexDirection: 'row', alignItems: 'center', gap: 7, paddingLeft: 6},
  drawerGroupIcon: {width: 27, height: 27, borderRadius: 7, alignItems: 'center', justifyContent: 'center', borderWidth: 1, borderColor: palette.border, backgroundColor: palette.panel},
  drawerGroupBody: {flex: 1, minWidth: 0, gap: 1},
  drawerGroupTitle: {color: palette.text, fontSize: 12.5, fontWeight: '700'},
  drawerGroupSubtitle: {color: palette.muted, fontSize: 9.5},
  drawerGroupInfo: {width: 36, height: 40, alignItems: 'center', justifyContent: 'center', borderRadius: 7},
  drawerAttention: {minWidth: 18, height: 18, paddingHorizontal: 5, borderRadius: 6, overflow: 'hidden', backgroundColor: palette.accentSoft, color: palette.warning, fontSize: 9, lineHeight: 18, fontWeight: '800', textAlign: 'center'},
  drawerProjectDetails: {marginHorizontal: 7, marginBottom: 5, padding: 9, gap: 4, borderWidth: 1, borderColor: palette.border, borderRadius: 8, backgroundColor: palette.input},
  drawerProjectLabel: {color: palette.muted, fontSize: 8.5, fontWeight: '800', letterSpacing: 1.05},
  drawerProjectPath: {color: palette.secondary, fontSize: 10.5, lineHeight: 15},
  smallDanger: {color: palette.danger},
  button: {minHeight: 44, paddingHorizontal: 13, flexDirection: 'row', gap: 7, alignItems: 'center', justifyContent: 'center', borderRadius: 8, backgroundColor: palette.accent},
  buttonQuiet: {backgroundColor: palette.raised, borderWidth: 1, borderColor: palette.borderStrong},
  buttonDanger: {backgroundColor: palette.dangerSurface, borderWidth: 1, borderColor: palette.dangerBorder},
  buttonText: {color: palette.accentInk, fontSize: 12.5, fontWeight: '700'},
  buttonQuietText: {color: palette.text},
  buttonDangerText: {color: palette.danger},
  disabled: {opacity: 0.36},
  chatHeader: {minHeight: 42, flexDirection: 'row', alignItems: 'center', gap: 7, borderBottomWidth: StyleSheet.hairlineWidth, borderBottomColor: palette.border},
  back: {color: palette.secondary, fontSize: 12.5, fontWeight: '600'},
  chatTitle: {color: palette.text, fontSize: 14.5, fontWeight: '700', flex: 1},
  chatRunState: {flexDirection: 'row', alignItems: 'center', gap: 5, paddingHorizontal: 7, height: 24, borderRadius: 7, borderWidth: 1, borderColor: palette.border, backgroundColor: palette.panel},
  chatRunText: {color: palette.muted, fontSize: 11, fontWeight: '800', letterSpacing: 0.5},
  chip: {maxWidth: '56%', flexShrink: 1, height: 29, flexDirection: 'row', alignItems: 'center', gap: 5, backgroundColor: 'transparent', borderColor: palette.border, borderWidth: 1, borderRadius: 7, paddingHorizontal: 8},
  warningChip: {borderColor: palette.warning},
  chipText: {color: palette.secondary, fontSize: 10.5, fontWeight: '600'},
  pickerModal: {flex: 1, justifyContent: 'flex-end'},
  pickerSheet: {maxHeight: '88%', minHeight: '54%', paddingHorizontal: 12, paddingBottom: 8, borderTopLeftRadius: Math.max(16, cardRadius + 6), borderTopRightRadius: Math.max(16, cardRadius + 6), borderWidth: 1, borderBottomWidth: 0, borderColor: palette.glassEdge, backgroundColor: palette.popover, overflow: 'hidden'},
  pickerSheetCompact: {minHeight: 0, maxHeight: '50%'},
  sheetHandle: {width: 36, height: 4, borderRadius: 2, backgroundColor: palette.borderStrong, alignSelf: 'center', marginTop: 8, marginBottom: 10},
  pickerHeader: {flexDirection: 'row', alignItems: 'center', gap: 10, paddingHorizontal: 3, paddingBottom: 10},
  pickerTitle: {color: palette.text, fontSize: 18, fontWeight: '700', letterSpacing: -0.45},
  pickerSubtitle: {color: palette.muted, fontSize: 10.5, marginTop: 1},
  pickerClose: {width: 34, height: 34, borderRadius: 8, alignItems: 'center', justifyContent: 'center'},
  pickerSearch: {minHeight: 40, flexDirection: 'row', alignItems: 'center', gap: 8, paddingHorizontal: 10, borderRadius: 8, borderWidth: 1, borderColor: palette.borderStrong, backgroundColor: palette.input, marginBottom: 8},
  pickerSearchInput: {flex: 1, color: palette.text, fontFamily, fontSize: 13, paddingVertical: 7},
  pickerSearchClear: {width: 28, height: 28, alignItems: 'center', justifyContent: 'center'},
  pickerListView: {flexGrow: 1},
  pickerList: {flexGrow: 1, paddingBottom: 8, gap: 2},
  pickerGroupHeader: {minHeight: 34, flexDirection: 'row', alignItems: 'center', gap: 7, paddingHorizontal: 8, paddingTop: 4, paddingBottom: 2},
  pickerGroupTitle: {flex: 1, color: palette.muted, fontSize: 9, fontWeight: '800', letterSpacing: 1.2},
  pickerGroupCount: {color: palette.muted, fontSize: 10, fontWeight: '700'},
  pickerRow: {minHeight: 52, flexDirection: 'row', alignItems: 'center', gap: 3, paddingHorizontal: 5, paddingVertical: 3, borderRadius: 8, borderWidth: 1, borderColor: 'transparent', backgroundColor: 'transparent'},
  pickerRowSelected: {borderColor: palette.accentBorder, backgroundColor: palette.accentSoft},
  pickerRowMain: {flex: 1, minWidth: 0, minHeight: 44, flexDirection: 'row', alignItems: 'center', gap: 9, paddingHorizontal: 4},
  pickerRowIcon: {width: 24, height: 30, flexShrink: 0, alignItems: 'center', justifyContent: 'center'},
  pickerRowIconSelected: {borderColor: palette.accent},
  pickerRowBody: {flex: 1, minWidth: 0, gap: 3},
  pickerRowTopline: {flexDirection: 'row', alignItems: 'center', gap: 8},
  pickerRowTitle: {flex: 1, color: palette.text, fontSize: 13, fontWeight: '600'},
  pickerRowMeta: {flexDirection: 'row', alignItems: 'center', gap: 6, minHeight: 14},
  pickerRowRoute: {flex: 1, color: palette.muted, fontSize: 9.5, lineHeight: 14},
  pickerRowDescription: {color: palette.secondary, fontSize: 10.5, lineHeight: 15},
  capabilityRow: {flexDirection: 'row', alignItems: 'center', height: 14, gap: 4},
  pickerCapabilityIcon: {width: 14, height: 14, alignItems: 'center', justifyContent: 'center'},
  pickerCapabilityFastIcon: {transform: [{translateY: -1}]},
  capabilityTag: {color: palette.secondary, fontSize: 8, fontWeight: '800', letterSpacing: 0.6, paddingHorizontal: 5, paddingVertical: 2, borderRadius: 4, overflow: 'hidden', backgroundColor: palette.input, borderWidth: StyleSheet.hairlineWidth, borderColor: palette.border},
  pickerRowFavorite: {width: 38, height: 44, alignItems: 'center', justifyContent: 'center', borderRadius: 7},
  pickerRowEffort: {minWidth: 36, height: 44, paddingHorizontal: 6, flexDirection: 'row', alignItems: 'center', justifyContent: 'center', gap: 4, borderRadius: 7},
  pickerRowEffortText: {color: palette.accent, fontSize: 9, fontWeight: '700'},
  reasoningEffortChoices: {marginLeft: 37, marginRight: 5, marginBottom: 3, paddingVertical: 6, gap: 5, borderTopWidth: StyleSheet.hairlineWidth, borderTopColor: palette.border},
  reasoningEffortLabel: {color: palette.muted, fontSize: 8.5, fontWeight: '800', letterSpacing: 0.8, textTransform: 'uppercase'},
  reasoningEffortScroll: {gap: 5, paddingRight: 8},
  reasoningEffortChoice: {minHeight: 38, minWidth: 48, paddingHorizontal: 10, alignItems: 'center', justifyContent: 'center', borderRadius: 7, borderWidth: 1, borderColor: palette.border, backgroundColor: palette.input},
  reasoningEffortChoiceSelected: {borderColor: palette.accentBorder, backgroundColor: palette.accentSoft},
  reasoningEffortChoiceText: {color: palette.secondary, fontSize: 10.5, fontWeight: '700'},
  reasoningEffortChoiceTextSelected: {color: palette.accent},
  pickerFavoritesOnly: {minHeight: 46, flexDirection: 'row', alignItems: 'center', gap: 8, paddingHorizontal: 8, borderTopWidth: StyleSheet.hairlineWidth, borderTopColor: palette.border},
  pickerFavoritesOnlyText: {flex: 1, color: palette.secondary, fontSize: 11, fontWeight: '600'},
  pickerSwitch: {width: 34, height: 20, padding: 2, borderRadius: 10, justifyContent: 'center', backgroundColor: palette.raised, borderWidth: 1, borderColor: palette.borderStrong},
  pickerSwitchOn: {backgroundColor: palette.accentSoft, borderColor: palette.accent},
  pickerSwitchThumb: {width: 14, height: 14, borderRadius: 7, backgroundColor: palette.muted},
  pickerSwitchThumbOn: {alignSelf: 'flex-end', backgroundColor: palette.accent},
  chatBody: {flex: 1, marginHorizontal: -12, position: 'relative'},
  transcriptMask: {flex: 1},
  messageList: {flex: 1},
  messageContent: {flexGrow: 1, paddingHorizontal: 14, paddingTop: 12, paddingBottom: 18, gap: 12},
  historyControl: {minHeight: 44, alignSelf: 'center', flexDirection: 'row', alignItems: 'center', gap: 6, paddingHorizontal: 12, borderRadius: 8, borderWidth: 1, borderColor: palette.border, backgroundColor: palette.panel},
  historyText: {color: palette.secondary, fontSize: 11, fontWeight: '600'},
  latestDock: {position: 'absolute', right: 0, left: 0, alignItems: 'center'},
  latestButton: {marginBottom: 8, minHeight: 44, flexDirection: 'row', alignItems: 'center', gap: 6, paddingHorizontal: 13, borderRadius: 9, borderWidth: 1, borderColor: palette.borderStrong, backgroundColor: palette.popover, shadowColor: '#000', shadowOpacity: 0.18, shadowRadius: 12, shadowOffset: {width: 0, height: 5}, elevation: 6},
  latestText: {color: palette.secondary, fontSize: 10.5, fontWeight: '700'},
  message: {borderRadius: 8, paddingHorizontal: 12, paddingVertical: 9, maxWidth: '82%'},
  userMessage: {backgroundColor: palette.accentSoft, borderWidth: 1, borderColor: palette.accentBorder, alignSelf: 'flex-end'},
  assistantMessage: {backgroundColor: 'transparent', alignSelf: 'stretch', maxWidth: '100%', paddingHorizontal: 2, paddingVertical: 4},
  messageRole: {color: palette.muted, fontSize: 10.5, fontWeight: '700', letterSpacing: 1, marginBottom: 5},
  modelChangeEvent: {alignSelf: 'stretch', minHeight: 48, flexDirection: 'row', alignItems: 'center', gap: 9, paddingVertical: 6},
  modelChangeLine: {flex: 1, height: StyleSheet.hairlineWidth, backgroundColor: palette.border},
  modelChangeCopy: {maxWidth: '76%', alignItems: 'center', gap: 2},
  modelChangeTopline: {flexDirection: 'row', alignItems: 'center', justifyContent: 'center', gap: 5},
  modelChangePrimary: {color: palette.secondary, fontSize: 12, lineHeight: 17},
  modelChangeModel: {color: palette.text, fontWeight: '700'},
  modelChangeDetail: {color: palette.muted, fontSize: 10.5, lineHeight: 15, textAlign: 'center'},
  reasoningBlock: {paddingBottom: 7, marginBottom: 6},
  reasoningHeader: {minHeight: 28, flexDirection: 'row', alignItems: 'center', gap: 6},
  reasoningLabel: {color: palette.muted, fontSize: 10.5, fontWeight: '600'},
  reasoning: {color: palette.muted, fontSize: 11.5, lineHeight: 17, fontStyle: 'italic', paddingTop: 3},
  activityGroup: {alignSelf: 'stretch', borderTopWidth: StyleSheet.hairlineWidth, borderBottomWidth: StyleSheet.hairlineWidth, borderColor: palette.border},
  activityGroupFailed: {borderColor: palette.dangerBorder},
  activitySummary: {minHeight: 48, flexDirection: 'row', alignItems: 'center', gap: 9, paddingHorizontal: 3, paddingVertical: 7},
  activitySummaryIcon: {width: 26, height: 26, flexShrink: 0, alignItems: 'center', justifyContent: 'center', borderRadius: 7, borderWidth: StyleSheet.hairlineWidth, borderColor: palette.border, backgroundColor: palette.panel},
  activitySummaryCopy: {flex: 1, minWidth: 0, gap: 2},
  activitySummaryLabel: {color: palette.text, fontSize: 13, lineHeight: 18, fontWeight: '600'},
  activitySummaryDetail: {color: palette.secondary, fontSize: 11.5, lineHeight: 16},
  activitySummaryStatus: {fontSize: 10.5, lineHeight: 15, fontWeight: '700'},
  activityRows: {paddingLeft: 17, paddingBottom: 8, borderLeftWidth: StyleSheet.hairlineWidth, borderLeftColor: palette.border, marginLeft: 16},
  runDetailsFooter: {gap: 5, paddingTop: 5, paddingRight: 4},
  runDetailsAction: {minHeight: 34, alignSelf: 'flex-start', flexDirection: 'row', alignItems: 'center', gap: 6, paddingHorizontal: 4},
  runDetailsActionText: {color: palette.muted, fontSize: 10.5, fontWeight: '600'},
  runDetailsError: {color: palette.danger, fontSize: 10.5, lineHeight: 15, paddingRight: 8},
  runDetailsPanel: {gap: 2, paddingTop: 5, paddingRight: 4},
  runDetailSection: {borderTopWidth: StyleSheet.hairlineWidth, borderTopColor: palette.border},
  runDetailSectionHeader: {minHeight: 38, flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', gap: 8, paddingHorizontal: 4},
  runDetailSectionLabel: {color: palette.secondary, fontSize: 11, lineHeight: 16, fontWeight: '600'},
  runDetailJson: {color: palette.muted, fontFamily: monoFamily, fontSize: 10, lineHeight: 15, paddingHorizontal: 4, paddingBottom: 9},
  activityRow: {minHeight: 46, flexDirection: 'row', gap: 8, paddingVertical: 7, paddingRight: 3},
  activityRowIcon: {width: 22, height: 22, flexShrink: 0, alignItems: 'center', justifyContent: 'center'},
  activityRowBody: {flex: 1, minWidth: 0, gap: 3},
  activityRowTopline: {flexDirection: 'row', alignItems: 'flex-start', gap: 8},
  activityRowLabel: {flex: 1, color: palette.text, fontSize: 12.5, lineHeight: 17, fontWeight: '600'},
  activityRowStatus: {fontSize: 10.5, lineHeight: 16, fontWeight: '600'},
  activityDetailScroll: {minWidth: '100%', paddingRight: 14},
  activityRowDetail: {color: palette.muted, fontFamily: monoFamily, fontSize: 11, lineHeight: 16},
  diffChips: {flexDirection: 'row', alignItems: 'center', gap: 5, marginTop: 2},
  diffChip: {paddingHorizontal: 6, paddingVertical: 2, borderRadius: 5, overflow: 'hidden', fontFamily: monoFamily, fontSize: 10.5, fontWeight: '700'},
  diffChipAdded: {color: palette.success, backgroundColor: palette.raised},
  diffChipRemoved: {color: palette.danger, backgroundColor: palette.dangerSurface},
  activityNotice: {minHeight: 48, alignSelf: 'stretch', flexDirection: 'row', alignItems: 'center', gap: 9, paddingHorizontal: 4, paddingVertical: 8, borderTopWidth: StyleSheet.hairlineWidth, borderBottomWidth: StyleSheet.hairlineWidth, borderColor: palette.border},
  missing: {color: palette.warning, fontSize: 12, paddingVertical: 6},
  attachments: {gap: 5, paddingTop: 6, paddingBottom: 2},
  attachment: {height: 27, maxWidth: 210, flexDirection: 'row', alignItems: 'center', gap: 5, backgroundColor: palette.raised, borderWidth: 1, borderColor: palette.border, borderRadius: 7, paddingHorizontal: 7},
  attachmentText: {maxWidth: 155, color: palette.secondary, fontSize: 10},
  composerDock: {position: 'absolute', right: 12, bottom: 0, left: 12, overflow: 'hidden'},
  composerLayer: {position: 'absolute', top: 0, right: 0, left: 0, zIndex: 1, paddingTop: 5, paddingBottom: 3},
  composerLayerFront: {zIndex: 2},
  composer: {backgroundColor: 'transparent', borderWidth: 1, borderColor: palette.borderStrong, borderRadius: inputRadius, padding: 8},
  compactComposer: {minHeight: 48, flexDirection: 'row', alignItems: 'center', gap: 4, padding: 4, borderWidth: 1, borderColor: palette.borderStrong, borderRadius: inputRadius, backgroundColor: 'transparent'},
  compactComposerPrompt: {flex: 1, minHeight: 38, flexDirection: 'row', alignItems: 'center', gap: 8, paddingHorizontal: 10, borderRadius: Math.max(6, inputRadius - 3)},
  compactComposerText: {flex: 1, color: palette.placeholder, fontSize: 13, fontWeight: '500'},
  composerContextRow: {minHeight: 33, flexDirection: 'row', alignItems: 'center', gap: 6, paddingBottom: 5, marginBottom: 3, borderBottomWidth: StyleSheet.hairlineWidth, borderBottomColor: palette.border},
  composerInput: {color: palette.text, minHeight: 46, maxHeight: 126, paddingHorizontal: 2, paddingTop: 5, paddingBottom: 4, fontFamily, fontSize: 14, lineHeight: 20, textAlignVertical: 'top'},
  composerActions: {height: 44, flexDirection: 'row', alignItems: 'center', gap: 4},
  composerSpacer: {flex: 1},
  iconButton: {width: 44, height: 44, borderRadius: 8, alignItems: 'center', justifyContent: 'center', backgroundColor: 'transparent'},
  iconButtonAccent: {backgroundColor: palette.accent},
  actionSheetList: {paddingBottom: 8, gap: 2},
  actionSheetButton: {minHeight: 48, flexDirection: 'row', alignItems: 'center', gap: 10, paddingHorizontal: 8, borderRadius: 8},
  actionSheetIcon: {width: 30, height: 30, alignItems: 'center', justifyContent: 'center'},
  actionSheetLabel: {color: palette.text, fontSize: 13.5, fontWeight: '600'},
  attentionCard: {backgroundColor: palette.panel, borderWidth: 1, borderColor: palette.border, borderRadius: cardRadius, padding: 14, gap: 10},
  inlineApprovalCard: {alignSelf: 'stretch', borderColor: palette.warning, backgroundColor: palette.input, borderRadius: Math.max(8, cardRadius - 2), padding: 12},
  attentionTitle: {color: palette.text, fontSize: 16, fontWeight: '600'},
  codeBlock: {color: palette.text, backgroundColor: palette.bg, borderRadius: inputRadius, padding: 10, fontFamily: monoFamily, fontSize: 12, maxHeight: 180},
  approvalDetailScroll: {minWidth: '100%'},
  actionRow: {flexDirection: 'row', justifyContent: 'flex-end', alignItems: 'center', gap: 8, flexWrap: 'wrap'},
  dialogBackdrop: {flex: 1, justifyContent: 'center', padding: 24, backgroundColor: 'rgba(0, 0, 0, 0.72)'},
  dialogCard: {gap: 14, padding: 18, borderRadius: cardRadius, borderWidth: 1, borderColor: palette.borderStrong, backgroundColor: palette.panel},
  hostList: {paddingBottom: 24, gap: 12},
  pageHeadingRow: {minHeight: 54, flexDirection: 'row', alignItems: 'center', gap: 4},
  pageHeadingCopy: {flex: 1, gap: 1},
  hostPairAction: {height: 34, flexDirection: 'row', alignItems: 'center', gap: 6, paddingHorizontal: 11, borderRadius: 8, borderWidth: 1, borderColor: palette.borderStrong, backgroundColor: palette.panel},
  hostPairActionText: {color: palette.secondary, fontSize: 11.5, fontWeight: '700'},
  hostPrimaryCard: {padding: 12, borderRadius: cardRadius, borderWidth: 1, borderColor: palette.focus, backgroundColor: palette.panel, gap: 11, shadowColor: '#000', shadowOpacity: 0.12, shadowRadius: 18, shadowOffset: {width: 0, height: 7}, elevation: 5},
  hostPrimaryTopline: {flexDirection: 'row', alignItems: 'center', gap: 9},
  hostMark: {width: 34, height: 34, resizeMode: 'contain'},
  hostPrimaryBody: {flex: 1, minWidth: 0, gap: 2},
  hostName: {color: palette.text, fontSize: 14, fontWeight: '700'},
  hostEndpoint: {color: palette.secondary, fontSize: 10.5},
  hostStatusPill: {height: 23, flexDirection: 'row', alignItems: 'center', gap: 5, paddingHorizontal: 7, borderRadius: 7, borderWidth: 1, borderColor: palette.border, backgroundColor: palette.input},
  hostStatusPillOnline: {borderColor: palette.borderStrong},
  hostStatusText: {color: palette.secondary, fontSize: 8, fontWeight: '800', textTransform: 'uppercase', letterSpacing: 0.55},
  hostMenu: {width: 32, height: 34, alignItems: 'center', justifyContent: 'center', borderRadius: 7},
  hostPrimaryFooter: {minHeight: 27, flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', gap: 8, paddingTop: 9, borderTopWidth: StyleSheet.hairlineWidth, borderTopColor: palette.border},
  hostActivity: {color: palette.muted, fontSize: 10},
  hostCompatibility: {flexDirection: 'row', alignItems: 'center', gap: 5},
  hostCompatibilityText: {color: palette.secondary, fontSize: 9.5, fontWeight: '600'},
  hostSection: {gap: 6},
  hostSectionLabel: {color: palette.muted, fontSize: 8.5, fontWeight: '800', letterSpacing: 1.2, paddingHorizontal: 2},
  hostRows: {gap: 3},
  hostRow: {minHeight: 54, flexDirection: 'row', alignItems: 'center', gap: 9, paddingHorizontal: 10, borderRadius: 9, borderWidth: 1, borderColor: palette.border, backgroundColor: palette.panel},
  hostRowMark: {width: 28, height: 28, resizeMode: 'contain'},
  hostRowBody: {flex: 1, minWidth: 0, gap: 2},
  hostRowTitle: {color: palette.text, fontSize: 12.5, fontWeight: '600'},
  hostRowMeta: {color: palette.muted, fontSize: 9.5},
  hostSwitchText: {color: palette.secondary, fontSize: 10, fontWeight: '700'},
  hostDisclosure: {minHeight: 58, flexDirection: 'row', alignItems: 'center', gap: 10, paddingHorizontal: 10, borderRadius: 9, borderWidth: 1, borderColor: palette.border, backgroundColor: palette.panel},
  hostDisclosureIcon: {width: 30, height: 30, borderRadius: 7, alignItems: 'center', justifyContent: 'center', borderWidth: 1, borderColor: palette.border, backgroundColor: palette.input},
  hostDisclosureBody: {flex: 1, minWidth: 0, gap: 2},
  hostDisclosureTitle: {color: palette.text, fontSize: 12.5, fontWeight: '600'},
  hostDisclosureCopy: {color: palette.muted, fontSize: 10, lineHeight: 14},
  hostManualForm: {gap: 9, paddingHorizontal: 9, paddingTop: 4, paddingBottom: 2},
  hostManualActions: {alignSelf: 'flex-end'},
  hostInfoRow: {minHeight: 58, flexDirection: 'row', alignItems: 'center', gap: 10, paddingHorizontal: 10, borderRadius: 9, borderWidth: StyleSheet.hairlineWidth, borderColor: palette.border, backgroundColor: palette.panel},
  hostDetails: {gap: 9, paddingBottom: 8},
  hostDetailBlock: {gap: 5, padding: 10, borderRadius: 8, borderWidth: 1, borderColor: palette.border, backgroundColor: palette.input},
  hostDetailValue: {color: palette.secondary, fontSize: 11, lineHeight: 16},
  hostDetailGrid: {flexDirection: 'row', gap: 7},
  hostDetailStat: {flex: 1, minHeight: 58, gap: 5, padding: 9, borderRadius: 8, borderWidth: 1, borderColor: palette.border, backgroundColor: palette.input},
  hostDetailStatValue: {color: palette.text, fontSize: 11, lineHeight: 15, fontWeight: '600'},
  section: {paddingVertical: 10, gap: 10},
  sectionTitle: {color: palette.text, fontSize: 16, fontWeight: '600'},
  help: {color: palette.muted, fontSize: 12.5, lineHeight: 18},
  empty: {flex: 1, minHeight: 180, paddingVertical: 38, paddingHorizontal: 16, alignItems: 'center', justifyContent: 'center', gap: 9},
  emptyTitle: {color: palette.text, fontSize: 17, fontWeight: '600'},
  emptyCopy: {maxWidth: 340, textAlign: 'center'},
  onboarding: {flex: 1, backgroundColor: palette.bg},
  onboardingTopbar: {height: 58, paddingHorizontal: 16, flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', backgroundColor: palette.sidebar, borderBottomWidth: StyleSheet.hairlineWidth, borderBottomColor: palette.border},
  onboardingBrand: {flexDirection: 'row', alignItems: 'center', gap: 9},
  onboardingMark: {width: 32, height: 32, resizeMode: 'contain'},
  onboardingWordmark: {color: palette.text, fontSize: 17, fontWeight: '700', letterSpacing: -0.55},
  mobileLabel: {color: palette.muted, fontSize: 9, fontWeight: '700', letterSpacing: 1.6},
  onboardingContent: {width: '100%', maxWidth: 520, alignSelf: 'center', paddingHorizontal: 20, paddingTop: 34, paddingBottom: 28, gap: 18},
  onboardingIntro: {gap: 9},
  heroTitle: {color: palette.text, fontSize: 34, lineHeight: 38, fontWeight: '700', letterSpacing: -1.5},
  heroCopy: {color: palette.secondary, fontSize: 14, lineHeight: 20},
  nearbyPanel: {gap: 9, padding: 13, borderRadius: cardRadius, borderWidth: 1, borderColor: palette.border, backgroundColor: palette.panel},
  nearbyHeading: {minHeight: 36, flexDirection: 'row', alignItems: 'center', gap: 10},
  nearbyTitle: {color: palette.text, fontSize: 14.5, fontWeight: '600'},
  nearbySubtitle: {color: palette.muted, fontSize: 10.5, lineHeight: 15},
  nearbyRefresh: {width: 36, height: 36, alignItems: 'center', justifyContent: 'center', borderRadius: 8, borderWidth: 1, borderColor: palette.border, backgroundColor: palette.raised},
  nearbyHost: {minHeight: 58, flexDirection: 'row', alignItems: 'center', gap: 9, paddingHorizontal: 10, paddingVertical: 8, borderRadius: 9, borderWidth: 1, borderColor: palette.borderStrong, backgroundColor: palette.input},
  nearbyHostPending: {borderColor: palette.accent, backgroundColor: palette.raised},
  nearbyHostMark: {width: 30, height: 30, resizeMode: 'contain'},
  nearbyPairingSpinner: {width: 30, height: 30, alignItems: 'center', justifyContent: 'center'},
  nearbyHostBody: {flex: 1, minWidth: 0, gap: 2},
  nearbyHostName: {color: palette.text, fontSize: 12.5, fontWeight: '700'},
  nearbyHostEndpoint: {color: palette.muted, fontSize: 9.5},
  nearbyStatus: {flexDirection: 'row', alignItems: 'center', gap: 5, paddingHorizontal: 7, height: 24, borderRadius: 7, borderWidth: 1, borderColor: palette.border, backgroundColor: palette.raised},
  nearbyStatusText: {color: palette.secondary, fontSize: 8, fontWeight: '800', textTransform: 'uppercase', letterSpacing: 0.45},
  nearbyConnectAction: {flexDirection: 'row', alignItems: 'center', gap: 4, paddingLeft: 8},
  nearbyConnectText: {color: palette.text, fontSize: 10.5, fontWeight: '700'},
  nearbyEmpty: {color: palette.muted, fontSize: 11.5, lineHeight: 17},
  nearbyHelp: {color: palette.secondary, fontSize: 10.5, lineHeight: 15},
  manualPairToggle: {minHeight: 58, flexDirection: 'row', alignItems: 'center', gap: 10, paddingHorizontal: 13, paddingVertical: 10, borderRadius: cardRadius, borderWidth: 1, borderColor: palette.border, backgroundColor: palette.panel},
  manualPairIcon: {width: 34, height: 34, borderRadius: 8, alignItems: 'center', justifyContent: 'center', backgroundColor: palette.raised},
  manualPairTitle: {color: palette.text, fontSize: 13, fontWeight: '600'},
  manualPairCopy: {color: palette.muted, fontSize: 10.5, lineHeight: 14},
  pairingPanel: {gap: 10, padding: 15, borderRadius: cardRadius, borderWidth: 1, borderColor: palette.border, backgroundColor: palette.panel},
  panelHeading: {flexDirection: 'row', alignItems: 'center', gap: 10, marginBottom: 3},
  panelIcon: {width: 34, height: 34, borderRadius: 8, borderWidth: 1, borderColor: palette.borderStrong, backgroundColor: palette.raised, alignItems: 'center', justifyContent: 'center'},
  panelTitle: {color: palette.text, fontSize: 14.5, fontWeight: '600', marginBottom: 1},
  fieldLabel: {color: palette.muted, fontSize: 9, fontWeight: '700', letterSpacing: 1.2, marginTop: 2},
  pairActions: {flexDirection: 'row', justifyContent: 'flex-end', gap: 8, marginTop: 3},
  directNote: {flexDirection: 'row', alignItems: 'flex-start', gap: 9, paddingHorizontal: 2},
  directNoteText: {flex: 1, color: palette.muted, fontSize: 11.5, lineHeight: 17},
  claimInput: {minHeight: 78, textAlignVertical: 'top'},
  formError: {color: palette.danger, fontSize: 13, lineHeight: 18},
  scanner: {flex: 1, backgroundColor: '#000'},
  scannerOverlay: {flex: 1, justifyContent: 'space-between', alignItems: 'center', padding: 24},
  scannerTitle: {color: '#fff', fontSize: 19, fontWeight: '700', textShadowColor: '#000', textShadowRadius: 8},
  });
}

export default App;
