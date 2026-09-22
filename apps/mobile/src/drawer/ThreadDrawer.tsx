import React, {useCallback, useEffect, useMemo, useRef, useState} from 'react';
import {BackHandler, Keyboard, Modal, Pressable, RefreshControl, SectionList, StyleSheet, TextInput, View, useWindowDimensions} from 'react-native';
import {Gesture, GestureDetector} from 'react-native-gesture-handler';
import Animated, {useAnimatedStyle, useSharedValue, withTiming} from 'react-native-reanimated';
import {SafeAreaView} from 'react-native-safe-area-context';
import {scheduleOnRN} from 'react-native-worklets';
import {newCommandId} from '../control/client';
import {type JsonValue, type PendingApprovalV1, type ThreadSummaryV1} from '../control/types';
import type {MilimController} from '../controller/useMilimController';
import {filterThreadsByQuery, groupMobileThreads, unreadThreadIds, type MobileThreadGroup} from '../mobileUi';
import {MilimIcon} from '../ui/MilimIcon';
import {useAppTheme, Text} from '../ui/appTheme';
import {useReducedMotion, MotionPressable} from '../ui/motion';
import {DrawerBackdropFade} from '../ui/fades';
import {Button, ActionSheetButton, Empty} from '../ui/controls';
import {modelId, confirmDestructive, showError} from '../ui/dialogs';
import {haptics} from '../ui/haptics';
import {PickerSheetFrame} from '../ui/PickerSheetFrame';
import {DRAWER_EDGE_GESTURE_WIDTH, DRAWER_SWIPE_ACTIVATION_DISTANCE, DRAWER_SWIPE_COMMIT_DISTANCE, DRAWER_SWIPE_COMMIT_VELOCITY} from '../ui/constants';
import {useThreadReads} from './useThreadReads';

const OPEN_MS = 180;
const CLOSE_MS = 150;
// Gesture velocities are points per second; the shared constant is per millisecond.
const COMMIT_VELOCITY = DRAWER_SWIPE_COMMIT_VELOCITY * 1000;

// Wraps the app surface with the left-edge open gesture and renders the thread
// drawer above it. Dragging runs on the UI thread, so the drawer tracks the
// finger even while a streamed answer keeps the JS thread busy.
export const ThreadDrawerLayout = React.memo(function MemoizedThreadDrawerLayout({
  children,
  open,
  onOpenChange,
  hostId,
  threads,
  models,
  approvals,
  selectedThreadId,
  onSelect,
  onRefresh,
  command,
  execute,
}: {
  children: React.ReactElement;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  hostId: string;
  threads: ThreadSummaryV1[];
  models: JsonValue[];
  approvals: PendingApprovalV1[];
  selectedThreadId: string | null;
  onSelect: (id: string) => void;
  onRefresh: () => Promise<unknown>;
  command: MilimController['command'];
  execute: MilimController['execute'];
}) {
  const {styles} = useAppTheme();
  const reduced = useReducedMotion();
  const {width} = useWindowDimensions();
  const drawerWidth = Math.min(width * 0.9, 380);
  // 0 is closed, 1 is fully open.
  const progress = useSharedValue(0);
  // The list mounts on first use so a cold start does not pay for it.
  const [mounted, setMounted] = useState(open);

  const settle = useCallback((next: boolean) => {
    if (next) setMounted(true);
    haptics.selection();
    onOpenChange(next);
  }, [onOpenChange]);

  useEffect(() => {
    if (open) {
      setMounted(true);
      // However the drawer was opened, it must not sit above a raised keyboard.
      Keyboard.dismiss();
    }
    progress.value = withTiming(open ? 1 : 0, {duration: open ? OPEN_MS : CLOSE_MS});
  }, [open, progress]);

  useEffect(() => {
    if (!open) return;
    const subscription = BackHandler.addEventListener('hardwareBackPress', () => {
      onOpenChange(false);
      return true;
    });
    return () => subscription.remove();
  }, [onOpenChange, open]);

  const edgeGesture = useMemo(() => Gesture.Pan()
    .enabled(!open)
    .hitSlop({left: 0, width: DRAWER_EDGE_GESTURE_WIDTH})
    .activeOffsetX(DRAWER_SWIPE_ACTIVATION_DISTANCE)
    .failOffsetY([-12, 12])
    .onBegin(() => {
      'worklet';
      scheduleOnRN(setMounted, true);
    })
    .onUpdate(event => {
      'worklet';
      if (reduced) return;
      progress.value = Math.max(0, Math.min(1, event.translationX / drawerWidth));
    })
    .onEnd(event => {
      'worklet';
      const commit = event.translationX >= DRAWER_SWIPE_COMMIT_DISTANCE || event.velocityX >= COMMIT_VELOCITY;
      progress.value = withTiming(commit ? 1 : 0, {duration: commit ? OPEN_MS : CLOSE_MS});
      if (commit) scheduleOnRN(settle, true);
    }), [drawerWidth, open, progress, reduced, settle]);

  const dismissGesture = useMemo(() => Gesture.Pan()
    .enabled(open)
    .activeOffsetX(-DRAWER_SWIPE_ACTIVATION_DISTANCE)
    .failOffsetY([-12, 12])
    .onUpdate(event => {
      'worklet';
      if (reduced) return;
      progress.value = Math.max(0, Math.min(1, 1 + event.translationX / drawerWidth));
    })
    .onEnd(event => {
      'worklet';
      const commit = event.translationX <= -DRAWER_SWIPE_COMMIT_DISTANCE || event.velocityX <= -COMMIT_VELOCITY;
      progress.value = withTiming(commit ? 0 : 1, {duration: commit ? CLOSE_MS : 120});
      if (commit) scheduleOnRN(settle, false);
    }), [drawerWidth, open, progress, reduced, settle]);

  const drawerStyle = useAnimatedStyle(() => ({
    transform: [{translateX: (progress.value - 1) * drawerWidth}],
  }), [drawerWidth]);
  const edgeFadeStyle = useAnimatedStyle(() => ({opacity: progress.value}));
  const close = useCallback(() => onOpenChange(false), [onOpenChange]);

  return (
    <>
      <GestureDetector gesture={edgeGesture}>{children}</GestureDetector>
      <View
        style={StyleSheet.absoluteFill}
        pointerEvents={open ? 'auto' : 'none'}
        accessibilityElementsHidden={!open}
        importantForAccessibility={open ? 'auto' : 'no-hide-descendants'}
        accessibilityViewIsModal={open}>
        {open ? (
          <Pressable style={StyleSheet.absoluteFill} onPress={close} accessibilityRole="button" accessibilityLabel="Close threads" />
        ) : null}
        <GestureDetector gesture={dismissGesture}>
          <Animated.View style={[styles.drawer, {width: drawerWidth}, drawerStyle]}>
            <Animated.View pointerEvents="none" style={[styles.drawerEdgeFade, edgeFadeStyle]}>
              <DrawerBackdropFade />
            </Animated.View>
            {mounted ? (
              <ThreadDrawerContent
                hostId={hostId}
                threads={threads}
                models={models}
                approvals={approvals}
                selectedThreadId={selectedThreadId}
                onSelect={onSelect}
                onRefresh={onRefresh}
                command={command}
                execute={execute}
                onClose={close}
              />
            ) : null}
          </Animated.View>
        </GestureDetector>
      </View>
    </>
  );
});

const ThreadDrawerContent = React.memo(function MemoizedThreadDrawerContent({
  hostId,
  threads,
  models,
  approvals,
  selectedThreadId,
  onSelect,
  onRefresh,
  command,
  execute,
  onClose,
}: {
  hostId: string;
  threads: ThreadSummaryV1[];
  models: JsonValue[];
  approvals: PendingApprovalV1[];
  selectedThreadId: string | null;
  onSelect: (id: string) => void;
  onRefresh: () => Promise<unknown>;
  command: MilimController['command'];
  execute: MilimController['execute'];
  onClose: () => void;
}) {
  const {palette, styles} = useAppTheme();
  const [title, setTitle] = useState('');
  const [query, setQuery] = useState('');
  const [refreshing, setRefreshing] = useState(false);
  const [renaming, setRenaming] = useState<ThreadSummaryV1 | null>(null);
  const [actionsFor, setActionsFor] = useState<ThreadSummaryV1 | null>(null);
  const [renameTitle, setRenameTitle] = useState('');
  const [collapsedGroups, setCollapsedGroups] = useState<Record<string, boolean>>({});
  const [detailsFor, setDetailsFor] = useState<MobileThreadGroup | null>(null);
  const initializedGroups = useRef(false);
  const reads = useThreadReads(hostId, threads, selectedThreadId);
  const unread = useMemo(() => unreadThreadIds(threads, reads, selectedThreadId), [reads, selectedThreadId, threads]);
  const approvalCounts = useMemo(() => approvals.reduce<Record<string, number>>((counts, approval) => {
    counts[approval.thread_id] = (counts[approval.thread_id] ?? 0) + 1;
    return counts;
  }, {}), [approvals]);
  const searching = Boolean(query.trim());
  const matchingThreads = useMemo(() => filterThreadsByQuery(threads, query), [query, threads]);
  const groups = useMemo(() => groupMobileThreads(matchingThreads, approvalCounts), [approvalCounts, matchingThreads]);
  const visibleCount = groups.reduce((count, group) => count + group.threads.length, 0);
  const defaultModel = modelId(models[0]) ?? '';
  // A search opens every group: a match hidden in a collapsed project is no match at all.
  const drawerSections = useMemo(() => groups.map(group => ({
    group,
    data: !searching && collapsedGroups[group.id] ? [] : group.threads,
  })), [collapsedGroups, groups, searching]);

  useEffect(() => {
    if (initializedGroups.current || !groups.length) return;
    const selectedGroup = groups.find(group => group.threads.some(thread => thread.id === selectedThreadId));
    const openGroupId = selectedGroup?.id ?? groups[0].id;
    setCollapsedGroups(Object.fromEntries(groups.map(group => [group.id, group.id !== openGroupId])));
    initializedGroups.current = true;
  }, [groups, selectedThreadId]);

  const create = async () => {
    const result = await command('thread.create', {
      title: title.trim() || 'New chat',
      settings: {model: defaultModel, privacy: 'off', toolApproval: 'review'},
    }, null);
    setTitle('');
    if (result.thread_id) onSelect(result.thread_id);
  };
  const refresh = async () => {
    setRefreshing(true);
    try {
      await onRefresh();
    } catch (error) {
      showError(error);
    } finally {
      setRefreshing(false);
    }
  };

  return (
    <>
      <SafeAreaView style={styles.drawerSafe} edges={['top', 'left', 'bottom']}>
        <View style={styles.drawerHeader}>
          <View style={styles.drawerHeading}>
            <Text style={styles.eyebrow}>THREADS</Text>
            <View style={styles.drawerTitleRow}>
              <Text style={styles.drawerTitle} accessibilityRole="header">Workspace</Text>
              <Text style={styles.drawerCount} accessibilityLabel={`${visibleCount} threads`}>{visibleCount}</Text>
            </View>
          </View>
          <MotionPressable style={styles.drawerClose} onPress={onClose} accessibilityLabel="Close threads">
            <MilimIcon name="x" size={17} color={palette.secondary} />
          </MotionPressable>
        </View>
        <View style={styles.drawerSearch}>
          <MilimIcon name="search" size={15} color={palette.muted} />
          <TextInput
            style={styles.drawerSearchInput}
            value={query}
            onChangeText={setQuery}
            placeholder="Search threads or projects"
            placeholderTextColor={palette.placeholder}
            accessibilityLabel="Search threads"
            autoCapitalize="none"
            autoCorrect={false}
            returnKeyType="search"
            clearButtonMode="never"
          />
          {query ? (
            <MotionPressable style={styles.pickerSearchClear} onPress={() => setQuery('')} hitSlop={8} accessibilityLabel="Clear search">
              <MilimIcon name="x" size={13} color={palette.muted} />
            </MotionPressable>
          ) : null}
        </View>
        {searching ? null : (
          <View style={styles.drawerCreateRow}>
            <TextInput
              style={[styles.input, styles.flex]}
              value={title}
              onChangeText={setTitle}
              placeholder="New chat"
              placeholderTextColor={palette.placeholder}
              accessibilityLabel="New thread title"
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
        )}
        <SectionList
          style={styles.drawerGroups}
          contentContainerStyle={styles.drawerGroupList}
          sections={drawerSections}
          keyExtractor={item => item.id}
          stickySectionHeadersEnabled={false}
          keyboardShouldPersistTaps="handled"
          keyboardDismissMode="on-drag"
          initialNumToRender={18}
          windowSize={7}
          refreshControl={
            <RefreshControl refreshing={refreshing} onRefresh={() => void refresh()} tintColor={palette.muted} colors={[palette.accent]} />
          }
          renderSectionFooter={() => <View style={styles.drawerGroupSeparator} />}
          ListEmptyComponent={searching
            ? <Empty title="No matching threads" copy="Try part of a title or a project folder name." />
            : <Empty title="No threads yet" copy="Create one here or start from an Agent on desktop." />}
          renderSectionHeader={({section}) => {
            const group = section.group;
            const collapsed = !searching && Boolean(collapsedGroups[group.id]);
            const groupUnread = group.threads.filter(thread => unread.has(thread.id)).length;
            return (
              <View style={styles.drawerGroup}>
                <View style={styles.drawerGroupHeader}>
                  <MotionPressable
                    style={styles.drawerGroupToggle}
                    disabled={searching}
                    onPress={() => setCollapsedGroups(current => ({...current, [group.id]: !collapsed}))}
                    accessibilityState={{expanded: !collapsed}}
                    accessibilityLabel={[
                      group.label,
                      `${group.threads.length} threads`,
                      group.busy ? 'running' : null,
                      group.attentionCount ? `${group.attentionCount} need attention` : null,
                      groupUnread ? `${groupUnread} unread` : null,
                    ].filter(Boolean).join(', ')}>
                    <MilimIcon name={collapsed ? 'chevron-down' : 'chevron-up'} size={13} color={palette.muted} />
                    <View style={styles.drawerGroupIcon}>
                      <MilimIcon name={group.workspace ? 'folder' : 'sparkles'} size={14} color={palette.secondary} />
                    </View>
                    <View style={styles.drawerGroupBody}>
                      <Text style={styles.drawerGroupTitle} numberOfLines={1}>{group.label}</Text>
                      <Text style={styles.drawerGroupSubtitle}>{group.subtitle} · {group.threads.length}</Text>
                    </View>
                    {group.busy ? <View style={[styles.dot, styles.dotOnline]} /> : null}
                    {collapsed && groupUnread ? <View style={styles.unreadDot} /> : null}
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
              unread={unread.has(item.id)}
              onOpen={() => onSelect(item.id)}
              onMenu={() => setActionsFor(item)}
            />
          )}
        />
      </SafeAreaView>
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
            <Text style={styles.attentionTitle} accessibilityRole="header">Rename thread</Text>
            <TextInput
              style={styles.input}
              value={renameTitle}
              onChangeText={setRenameTitle}
              autoFocus
              maxLength={160}
              placeholder="Thread title"
              placeholderTextColor={palette.placeholder}
              accessibilityLabel="Thread title"
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
    </>
  );
});

function ThreadCard({thread, attentionCount, selected, unread, onOpen, onMenu}: {thread: ThreadSummaryV1; attentionCount: number; selected: boolean; unread: boolean; onOpen: () => void; onMenu: () => void}) {
  const {palette, styles} = useAppTheme();
  return (
    <Pressable
      style={[styles.threadCard, selected && styles.threadCardSelected]}
      onPress={onOpen}
      onLongPress={() => {
        haptics.selection();
        onMenu();
      }}
      accessibilityRole="button"
      accessibilityState={{selected}}
      accessibilityLabel={[
        thread.title,
        unread ? 'unread' : null,
        thread.busy ? 'running' : null,
        attentionCount ? `${attentionCount} need attention` : null,
        thread.origin?.kind === 'schedule' ? 'scheduled' : null,
      ].filter(Boolean).join(', ')}>
      <View style={styles.threadTopline}>
        {unread ? <View style={styles.unreadDot} /> : null}
        <Text style={[styles.threadTitle, unread && styles.threadTitleUnread]} numberOfLines={1}>{thread.title}</Text>
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
