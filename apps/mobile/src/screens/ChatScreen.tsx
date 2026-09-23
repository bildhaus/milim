import React, {useCallback, useEffect, useMemo, useRef, useState} from 'react';
import {Animated, View} from 'react-native';
import {MAX_ATTACHMENTS, pickFiles, pickPhoto} from '../attachments';
import type {ControlAttachmentV1} from '../control/types';
import {useHotState} from '../controller/hotStore';
import type {MilimController} from '../controller/useMilimController';
import {mobileModelOptions} from '../modelPicker';
import {canUseCompactComposer} from '../mobileUi';
import {useAppTheme, Text} from '../ui/appTheme';
import {useReducedMotion} from '../ui/motion';
import {ActionSheetButton, Empty} from '../ui/controls';
import {showError} from '../ui/dialogs';
import {PickerSheetFrame} from '../ui/PickerSheetFrame';
import {Transcript, type TranscriptHandle} from '../transcript/Transcript';
import {ComposerDock} from '../composer/ComposerDock';
import {ModelPickerSheet} from '../pickers/ModelPickerSheet';
import {AgentPickerSheet} from '../pickers/AgentPickerSheet';

export type ChatController = Pick<MilimController,
  | 'activeHost'
  | 'acceptedRetry'
  | 'pendingRetry'
  | 'bootstrap'
  | 'command'
  | 'execute'
  | 'hot'
  | 'loadMoreRunEvents'
  | 'loadRunDetails'
  | 'prepareAttachments'
  | 'refreshTimeline'
  | 'selectedThreadId'
  | 'setDraft'
  | 'status'
>;

// Owns the layout state the transcript and composer share. It subscribes to
// neither the timeline nor the draft text, so it stays idle while an answer
// streams or the user types.
export const ChatScreen = React.memo(function MemoizedChatScreen({controller, openThreads}: {controller: ChatController; openThreads: () => void}) {
  const {styles} = useAppTheme();
  const reduced = useReducedMotion();
  const thread = controller.bootstrap?.threads.find(item => item.id === controller.selectedThreadId);
  const threadApprovals = useMemo(
    () => controller.bootstrap?.pending_approvals.filter(approval => approval.thread_id === thread?.id) ?? [],
    [controller.bootstrap?.pending_approvals, thread?.id],
  );
  const threadPendingInputs = useMemo(
    () => controller.bootstrap?.pending_inputs.filter(input => input.thread_id === thread?.id) ?? [],
    [controller.bootstrap?.pending_inputs, thread?.id],
  );
  const activeRun = useMemo(
    () => controller.bootstrap?.active_runs.find(run => run.thread_id === thread?.id) ?? null,
    [controller.bootstrap?.active_runs, thread?.id],
  );
  const hasDraft = useHotState(controller.hot, state => state.draft.trim().length > 0);
  const [attachments, setAttachments] = useState<ControlAttachmentV1[]>([]);
  const [modelPickerVisible, setModelPickerVisible] = useState(false);
  const [agentPickerVisible, setAgentPickerVisible] = useState(false);
  const [attachmentMenuVisible, setAttachmentMenuVisible] = useState(false);
  const [awayFromLatest, setAwayFromLatest] = useState(false);
  const [inputFocused, setInputFocused] = useState(false);
  const [forcedComposerOpen, setForcedComposerOpen] = useState(false);
  const [composerCompact, setComposerCompact] = useState(true);
  const [expandedComposerHeight, setExpandedComposerHeight] = useState(138);
  const [compactComposerHeight, setCompactComposerHeight] = useState(56);
  const transcript = useRef<TranscriptHandle>(null);
  const composerProgress = useRef(new Animated.Value(0)).current;
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
  const shouldCompactComposer = canUseCompactComposer({
    awayFromLatest,
    // Only emptiness matters here; the text itself stays inside the dock.
    draft: hasDraft ? 'draft' : '',
    attachmentCount: attachments.length,
    inputFocused,
    pendingApproval: threadApprovals.length > 0,
    forcedOpen: forcedComposerOpen,
  });
  const composerHeight = useMemo(() => composerProgress.interpolate({
    inputRange: [0, 1],
    outputRange: [expandedComposerHeight, compactComposerHeight],
  }), [compactComposerHeight, composerProgress, expandedComposerHeight]);

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
    setForcedComposerOpen(false);
    setInputFocused(false);
  }, [thread?.id]);

  const openModelPicker = useCallback(() => setModelPickerVisible(true), []);
  const openAgentPicker = useCallback(() => setAgentPickerVisible(true), []);
  const openAttachmentMenu = useCallback(() => setAttachmentMenuVisible(true), []);
  const followLatestAfterSend = useCallback(() => transcript.current?.followLatest(!reduced), [reduced]);

  if (!thread) {
    return <Empty title="Choose a thread" copy="Open the thread drawer to select or create a conversation." action="Open threads" onAction={openThreads} />;
  }
  const missingAgent = Boolean(
    thread.agent_id && !controller.bootstrap?.agents.some(agent => agent.id === thread.agent_id),
  );
  const activeAgent = controller.bootstrap?.agents.find(agent => agent.id === thread.agent_id) ?? null;
  const addAttachments = async (source: 'library' | 'camera' | 'file') => {
    setAttachmentMenuVisible(false);
    const items = source === 'file' ? await pickFiles() : await pickPhoto(source);
    setAttachments(current => [...current, ...items].slice(0, MAX_ATTACHMENTS));
  };
  return (
    <View style={styles.screen}>
      <View style={styles.chatHeader}>
        <Text style={styles.chatTitle} numberOfLines={1} accessibilityRole="header">{thread.title}</Text>
        {thread.origin?.kind === 'schedule' ? <Text style={styles.threadOrigin}>Scheduled</Text> : null}
        {thread.busy ? (
          <View style={styles.chatRunState}>
            <View style={[styles.dot, styles.dotOnline]} />
            <Text style={styles.chatRunText}>RUNNING</Text>
          </View>
        ) : null}
      </View>
      <View style={styles.chatBody}>
        <Transcript
          ref={transcript}
          controller={controller}
          threadId={thread.id}
          approvals={threadApprovals}
          pendingInputs={threadPendingInputs}
          composerHeight={composerHeight}
          composerInset={composerCompact ? compactComposerHeight : expandedComposerHeight}
          onAwayFromLatestChange={setAwayFromLatest}
        />
        <ComposerDock
          controller={controller}
          thread={thread}
          activeRun={activeRun}
          activeAgent={activeAgent}
          missingAgent={missingAgent}
          selectedModel={selectedModel}
          attachments={attachments}
          setAttachments={setAttachments}
          compact={composerCompact}
          forcedOpen={forcedComposerOpen}
          progress={composerProgress}
          height={composerHeight}
          onFocusChange={setInputFocused}
          onForcedOpenChange={setForcedComposerOpen}
          onCompactHeight={setCompactComposerHeight}
          onExpandedHeight={setExpandedComposerHeight}
          onOpenModelPicker={openModelPicker}
          onOpenAgentPicker={openAgentPicker}
          onOpenAttachmentMenu={openAttachmentMenu}
          onSent={followLatestAfterSend}
        />
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
