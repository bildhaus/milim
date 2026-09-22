import React, {useCallback, useEffect, useImperativeHandle, useMemo, useRef, useState} from 'react';
import {Animated, FlatList, Keyboard, Platform, View} from 'react-native';
import {projectTranscriptIncrementally, type ProjectedTranscriptItem, type TranscriptProjectionCache} from '../control/replica';
import type {PendingApprovalV1, PendingInputV1} from '../control/types';
import {useHotState} from '../controller/hotStore';
import type {MilimController} from '../controller/useMilimController';
import {mobilePerfMark, mobilePerfMeasure} from '../performance';
import {nextAwayFromLatest, shouldHoldCompactComposerForLatestReturn, transcriptDistanceFromLatest} from '../mobileUi';
import {MilimIcon} from '../ui/MilimIcon';
import {useAppTheme, Text} from '../ui/appTheme';
import {useReducedMotion, MotionPressable} from '../ui/motion';
import {TranscriptFadeOverlay} from '../ui/fades';
import {Empty} from '../ui/controls';
import {showError} from '../ui/dialogs';
import {TRANSCRIPT_FADE_HEIGHT} from '../ui/constants';
import {MessageActionsSheet, type MessageActionTarget} from './MessageActionsSheet';
import {TranscriptItemView} from './TranscriptItemView';

export type TranscriptHandle = {
  // Re-pins the view to the newest item, as after sending a message.
  followLatest: (animated: boolean) => void;
};

type TranscriptController = Pick<MilimController,
  | 'execute'
  | 'hot'
  | 'loadMoreRunEvents'
  | 'loadRunDetails'
  | 'refreshTimeline'
>;

// The only subscriber to the streamed timeline: a live answer re-renders this
// list and nothing else on the chat screen.
export const Transcript = React.memo(function MemoizedTranscript({
  ref,
  controller,
  threadId,
  approvals,
  pendingInputs,
  composerHeight,
  composerInset,
  onAwayFromLatestChange,
}: {
  ref: React.Ref<TranscriptHandle>;
  controller: TranscriptController;
  threadId: string;
  approvals: PendingApprovalV1[];
  pendingInputs: PendingInputV1[];
  composerHeight: Animated.AnimatedInterpolation<number>;
  composerInset: number;
  onAwayFromLatestChange: (away: boolean) => void;
}) {
  const {markdownStyles, palette, styles} = useAppTheme();
  const reduced = useReducedMotion();
  const timeline = useHotState(controller.hot, state => state.timeline);
  useEffect(() => {
    if (timeline?.threadId !== threadId) return;
    mobilePerfMark('thread.open.end');
    mobilePerfMeasure('thread.open', 'thread.open.start', 'thread.open.end');
  }, [threadId, timeline?.threadId]);
  const transcriptProjection = useRef<{
    threadId: string | null;
    epoch: string;
    cache: TranscriptProjectionCache | null;
  }>({threadId: null, epoch: '', cache: null});
  const transcriptItems = useMemo(() => {
    mobilePerfMark('transcript.project.start');
    const matchingTimeline = timeline?.threadId === threadId ? timeline : null;
    const epoch = matchingTimeline?.epoch ?? '';
    const previous = transcriptProjection.current;
    const cache = projectTranscriptIncrementally(
      previous.threadId === threadId && previous.epoch === epoch ? previous.cache : null,
      matchingTimeline?.items ?? [],
      approvals,
      pendingInputs,
    );
    mobilePerfMark('transcript.project.end');
    mobilePerfMeasure('transcript.project', 'transcript.project.start', 'transcript.project.end');
    transcriptProjection.current = {threadId, epoch, cache};
    return cache.projected;
  }, [approvals, pendingInputs, threadId, timeline]);
  const inspectableRunIds = useMemo(
    () => new Set(
      transcriptItems
        .filter(item => item.kind === 'message' && item.role === 'assistant' && item.ledgerVersion === 1 && item.runId)
        .map(item => (item as Extract<ProjectedTranscriptItem, {kind: 'message'}>).runId as string),
    ),
    [transcriptItems],
  );
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [showLatest, setShowLatest] = useState(false);
  const [messageActions, setMessageActions] = useState<MessageActionTarget | null>(null);
  const closeMessageActions = useCallback(() => setMessageActions(null), []);
  const messageList = useRef<FlatList<ProjectedTranscriptItem>>(null);
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
    onAwayFromLatestChange(value);
  }, [onAwayFromLatestChange]);

  useImperativeHandle(ref, () => ({
    followLatest(animated) {
      shouldScrollToLatest.current = true;
      followingLatest.current = true;
      returningToLatest.current = false;
      updateShowLatest(false);
      updateAwayFromLatest(false);
      scheduleLatestScroll(animated);
    },
  }), [scheduleLatestScroll, updateAwayFromLatest, updateShowLatest]);

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
  }, [threadId, updateAwayFromLatest, updateShowLatest]);

  const renderTranscriptItem = useCallback(({item}: {item: ProjectedTranscriptItem}) => (
    <TranscriptItemView
      item={item}
      markdownStyles={markdownStyles}
      execute={controller.execute}
      runDetailsEnabled={item.kind === 'activity' && inspectableRunIds.has(item.runId)}
      loadRunDetails={controller.loadRunDetails}
      loadMoreRunEvents={controller.loadMoreRunEvents}
      onMessageActions={setMessageActions}
    />
  ), [
    controller.execute,
    controller.loadMoreRunEvents,
    controller.loadRunDetails,
    inspectableRunIds,
    markdownStyles,
  ]);
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
  const transcriptBottomInset = useMemo(
    () => Animated.add(composerHeight, TRANSCRIPT_FADE_HEIGHT),
    [composerHeight],
  );

  return (
    <>
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
          ListHeaderComponent={timeline?.hasOlder ? (
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
          keyboardDismissMode={Platform.OS === 'ios' ? 'interactive' : 'none'}
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
              bottomInset: composerInset + TRANSCRIPT_FADE_HEIGHT,
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
      <MessageActionsSheet target={messageActions} onClose={closeMessageActions} />
    </>
  );
});
