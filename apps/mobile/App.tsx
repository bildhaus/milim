import React, {useCallback, useEffect, useMemo, useState} from 'react';
import {Image, Linking, Pressable, StatusBar, StyleSheet, View} from 'react-native';
import {GestureHandlerRootView} from 'react-native-gesture-handler';
import {KeyboardProvider} from 'react-native-keyboard-controller';
import {SafeAreaProvider, SafeAreaView} from 'react-native-safe-area-context';
import {cleanupStaleAttachments} from './src/attachments';
import {useMilimController} from './src/controller/useMilimController';
import {mobilePerfMark, mobilePerfMeasure, mobileStartupTiming} from './src/performance';
import {lowercaseMilimBrand} from './src/mobileUi';
import {MilimIcon} from './src/ui/MilimIcon';
import {appTheme, AppThemeContext, useAppTheme, Text} from './src/ui/appTheme';
import {MotionPressable, ScreenStage} from './src/ui/motion';
import {ThreadKeyboardAvoidingView} from './src/ui/ThreadKeyboardAvoidingView';
import {AppearanceBackground} from './src/ui/AppearanceBackground';
import {showError} from './src/ui/dialogs';
import {PairingScreen, PairingModal} from './src/screens/PairingScreen';
import {ThreadDrawerLayout} from './src/drawer/ThreadDrawer';
import {parseMilimLink, type MilimLink} from './src/deepLinks';
import {type ChatController, ChatScreen} from './src/screens/ChatScreen';
import {AttentionScreen} from './src/screens/AttentionScreen';
import {type HostsController, HostsScreen} from './src/screens/HostsScreen';
import {milimLogo} from './src/ui/assets';

type AppScreen = 'chat' | 'attention' | 'hosts';

mobilePerfMark('app.render.start');

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
  const [threadLinkTarget, setThreadLinkTarget] = useState<Extract<MilimLink, {kind: 'thread'}> | null>(null);
  const pairedHostIds = useMemo(
    () => controller.hosts.map(host => host.hostId),
    [controller.hosts],
  );
  const openThreads = useCallback(() => setThreadDrawerVisible(true), []);
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
    execute: controller.execute,
    loadMoreRunEvents: controller.loadMoreRunEvents,
    loadRunDetails: controller.loadRunDetails,
    prepareAttachments: controller.prepareAttachments,
    refreshTimeline: controller.refreshTimeline,
    selectedThreadId: controller.selectedThreadId,
    setDraft: controller.setDraft,
    status: controller.status,
    hot: controller.hot,
  }), [
    controller.activeHost,
    controller.acceptedRetry,
    controller.pendingRetry,
    controller.bootstrap,
    controller.command,
    controller.execute,
    controller.loadMoreRunEvents,
    controller.loadRunDetails,
    controller.prepareAttachments,
    controller.refreshTimeline,
    controller.selectedThreadId,
    controller.setDraft,
    controller.status,
    controller.hot,
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
  useEffect(() => {
    mobilePerfMark('app.render.end');
    mobilePerfMeasure('app.render', 'app.render.start', 'app.render.end');
    void mobileStartupTiming();
    void cleanupStaleAttachments();
    const openLink = (url: string | null) => {
      const link = url ? parseMilimLink(url) : null;
      if (!link) return;
      if (link.kind === 'thread') {
        setThreadLinkTarget(link);
        return;
      }
      setPairingClaim(link.claim);
      setPairingVisible(true);
    };
    void Linking.getInitialURL().then(openLink);
    const subscription = Linking.addEventListener('url', event => openLink(event.url));
    return () => subscription.remove();
  }, []);

  // A thread link may name another paired desktop: switch to it first, then open
  // the thread once that desktop's thread list has arrived.
  const {setActiveHost} = controller;
  const activeHostId = controller.activeHost?.hostId ?? null;
  const bootstrapHostId = controller.bootstrap?.host_id ?? null;
  const bootstrapThreads = controller.bootstrap?.threads;
  useEffect(() => {
    if (!threadLinkTarget || !activeHostId) return;
    const hostId = threadLinkTarget.hostId ?? activeHostId;
    if (hostId !== activeHostId) {
      if (pairedHostIds.includes(hostId)) setActiveHost(hostId);
      else setThreadLinkTarget(null);
      return;
    }
    if (bootstrapHostId !== activeHostId || !bootstrapThreads) return;
    if (bootstrapThreads.some(thread => thread.id === threadLinkTarget.threadId)) {
      selectThread(threadLinkTarget.threadId);
    }
    setThreadLinkTarget(null);
  }, [activeHostId, bootstrapHostId, bootstrapThreads, pairedHostIds, selectThread, setActiveHost, threadLinkTarget]);

  if (!controller.activeHost) {
    return (
      <AppThemeContext.Provider value={theme}>
        <SafeAreaProvider>
          <KeyboardProvider>
          <StatusBar barStyle={theme.isDark ? 'light-content' : 'dark-content'} />
          <PairingScreen
            claim={pairingClaim}
            setClaim={setPairingClaim}
            onPair={controller.pair}
            onPairNearby={controller.pairNearby}
            pairedHostIds={pairedHostIds}
          />
          </KeyboardProvider>
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
        <KeyboardProvider>
        <GestureHandlerRootView style={stylesForApp.gestureRoot}>
        <StatusBar barStyle={theme.isDark ? 'light-content' : 'dark-content'} />
        <ThreadDrawerLayout
          open={threadDrawerVisible}
          onOpenChange={setThreadDrawerVisible}
          hostId={controller.activeHost.hostId}
          threads={controller.bootstrap?.threads ?? emptyList}
          models={controller.bootstrap?.models ?? emptyList}
          approvals={controller.bootstrap?.pending_approvals ?? emptyList}
          selectedThreadId={controller.selectedThreadId}
          onSelect={selectThread}
          onRefresh={controller.refreshBootstrap}
          command={controller.command}
          execute={controller.execute}>
        <View style={styles.root}>
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
              <Image source={milimLogo} style={styles.brandMark} accessible={false} />
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
            accessibilityRole={controller.pendingRetry || controller.status === 'offline' ? 'button' : 'alert'}
            accessibilityLiveRegion="polite"
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
              hot={controller.hot}
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
        </ThreadDrawerLayout>
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
        </GestureHandlerRootView>
        </KeyboardProvider>
      </SafeAreaProvider>
    </AppThemeContext.Provider>
  );
}

// Stable empty lists keep the memoized drawer from re-rendering before bootstrap.
const emptyList: never[] = [];

const stylesForApp = StyleSheet.create({
  gestureRoot: {flex: 1},
});

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

export default App;
