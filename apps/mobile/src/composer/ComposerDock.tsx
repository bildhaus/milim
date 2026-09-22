import React, {useEffect, useRef, useState} from 'react';
import {Alert, Animated, ScrollView, TextInput, View} from 'react-native';
import {cleanupAttachments, promptWithAttachments} from '../attachments';
import type {AgentSummaryV1, ControlAttachmentV1, JsonValue, RunSnapshotV1, ThreadSummaryV1} from '../control/types';
import {useHotState} from '../controller/hotStore';
import type {MilimController} from '../controller/useMilimController';
import type {MobileModelOption} from '../modelPicker';
import {MilimIcon} from '../ui/MilimIcon';
import {AgentAvatar} from '../ui/AgentAvatar';
import {useAppTheme, Text} from '../ui/appTheme';
import {useReducedMotion, MotionPressable} from '../ui/motion';
import {IconButton} from '../ui/controls';
import {showError} from '../ui/dialogs';
import {haptics} from '../ui/haptics';
import {PickerChip} from '../pickers/PickerChip';

type ComposerController = Pick<MilimController,
  | 'acceptedRetry'
  | 'activeHost'
  | 'command'
  | 'hot'
  | 'pendingRetry'
  | 'prepareAttachments'
  | 'setDraft'
  | 'status'
>;

// The only subscriber to the draft: a keystroke re-renders the dock, not the
// transcript or the picker sheets.
export const ComposerDock = React.memo(function MemoizedComposerDock({
  controller,
  thread,
  activeRun,
  activeAgent,
  missingAgent,
  selectedModel,
  attachments,
  setAttachments,
  compact,
  forcedOpen,
  progress,
  height,
  onFocusChange,
  onForcedOpenChange,
  onCompactHeight,
  onExpandedHeight,
  onOpenModelPicker,
  onOpenAgentPicker,
  onOpenAttachmentMenu,
  onSent,
}: {
  controller: ComposerController;
  thread: ThreadSummaryV1;
  activeRun: RunSnapshotV1 | null;
  activeAgent: AgentSummaryV1 | null;
  missingAgent: boolean;
  selectedModel: MobileModelOption | null;
  attachments: ControlAttachmentV1[];
  setAttachments: React.Dispatch<React.SetStateAction<ControlAttachmentV1[]>>;
  compact: boolean;
  forcedOpen: boolean;
  progress: Animated.Value;
  height: Animated.AnimatedInterpolation<number>;
  onFocusChange: (focused: boolean) => void;
  onForcedOpenChange: (open: boolean) => void;
  onCompactHeight: (height: number) => void;
  onExpandedHeight: (height: number) => void;
  onOpenModelPicker: () => void;
  onOpenAgentPicker: () => void;
  onOpenAttachmentMenu: () => void;
  onSent: () => void;
}) {
  const {palette, styles} = useAppTheme();
  const reduced = useReducedMotion();
  const draft = useHotState(controller.hot, state => state.draft);
  const [busy, setBusy] = useState(false);
  const composerInput = useRef<React.ElementRef<typeof TextInput>>(null);

  const handledRetry = useRef<string | null>(null);
  useEffect(() => {
    const accepted = controller.acceptedRetry;
    if (!accepted || !['turn.send', 'turn.steer'].includes(accepted.command.kind) ||
      accepted.hostId !== controller.activeHost?.hostId ||
      accepted.command.thread_id !== thread.id || handledRetry.current === accepted.command.command_id) return;
    handledRetry.current = accepted.command.command_id;
    const payload = accepted.command.payload as {attachments?: {id: string}[]};
    const sentIds = new Set(payload.attachments?.map(item => item.id) ?? []);
    const sent = attachments.filter(item => sentIds.has(item.id));
    if (sent.length) {
      void cleanupAttachments(sent).catch(showError);
      setAttachments(current => current.filter(item => !sentIds.has(item.id)));
    }
  }, [attachments, controller.acceptedRetry, controller.activeHost?.hostId, setAttachments, thread.id]);

  useEffect(() => {
    if (!forcedOpen || compact) return;
    const focusTimer = setTimeout(() => {
      composerInput.current?.focus();
    }, reduced ? 0 : 160);
    return () => clearTimeout(focusTimer);
  }, [compact, forcedOpen, reduced]);

  const send = async () => {
    if (controller.status !== 'online') {
      Alert.alert('Saved as a draft', 'milim will not auto-send this prompt after reconnecting.');
      return;
    }
    haptics.send();
    setBusy(true);
    try {
      const wireAttachments = await controller.prepareAttachments(attachments);
      await controller.command(
        'turn.send',
        {
          text: promptWithAttachments(draft, attachments),
          display_text: draft,
          attachments: wireAttachments,
        } as unknown as JsonValue,
        thread.id,
        thread.revision,
      );
      controller.setDraft('');
      await cleanupAttachments(attachments);
      setAttachments([]);
      onSent();
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
    haptics.send();
    setBusy(true);
    try {
      const wireAttachments = await controller.prepareAttachments(attachments);
      await controller.command(
        'turn.steer',
        {
          run_id: activeRun.id,
          text: promptWithAttachments(draft, attachments),
          display_text: draft,
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
  const removeAttachment = async (attachment: ControlAttachmentV1) => {
    await cleanupAttachments([attachment]);
    setAttachments(current => current.filter(item => item.id !== attachment.id));
  };
  const stop = () => void controller.command('turn.stop', null, thread.id).catch(showError);
  const nothingToSend = !draft.trim() && !attachments.length;

  return (
    <Animated.View
      style={[
        styles.composerDock,
        {height},
      ]}>
      <Animated.View
        style={[
          styles.composerLayer,
          compact && styles.composerLayerFront,
          {
            opacity: progress,
            transform: [{
              translateY: progress.interpolate({inputRange: [0, 1], outputRange: [6, 0]}),
            }],
          },
        ]}
        pointerEvents={compact ? 'auto' : 'none'}
        accessibilityElementsHidden={!compact}
        importantForAccessibility={compact ? 'auto' : 'no-hide-descendants'}
        onLayout={({nativeEvent}) => onCompactHeight(nativeEvent.layout.height)}>
        {missingAgent ? <Text style={styles.missing}>This Agent was deleted. Clear or replace it before sending.</Text> : null}
        <View style={styles.compactComposer}>
          <MotionPressable
            style={styles.compactComposerPrompt}
            hitSlop={3}
            onPress={() => onForcedOpenChange(true)}
            accessibilityLabel="Expand message composer">
            {thread.busy ? <View style={[styles.dot, styles.dotOnline]} /> : <MilimIcon name="sparkles" size={13} color={palette.muted} />}
            <Text style={styles.compactComposerText} numberOfLines={1}>
              {thread.busy ? 'milim is working · Message milim…' : 'Message milim…'}
            </Text>
          </MotionPressable>
          {thread.busy ? (
            <IconButton icon="square" label="Stop generating" tone="quiet" onPress={stop} />
          ) : null}
        </View>
      </Animated.View>
      <Animated.View
        style={[
          styles.composerLayer,
          !compact && styles.composerLayerFront,
          {
            opacity: progress.interpolate({inputRange: [0, 1], outputRange: [1, 0]}),
            transform: [{
              translateY: progress.interpolate({inputRange: [0, 1], outputRange: [0, 8]}),
            }],
          },
        ]}
        pointerEvents={compact ? 'none' : 'auto'}
        accessibilityElementsHidden={compact}
        importantForAccessibility={compact ? 'no-hide-descendants' : 'auto'}
        onLayout={({nativeEvent}) => onExpandedHeight(nativeEvent.layout.height)}>
        {missingAgent ? <Text style={styles.missing}>This Agent was deleted. Clear or replace it before sending.</Text> : null}
        <View style={styles.composer}>
          <View style={styles.composerContextRow}>
            <PickerChip
              icon="sparkles"
              providerBrand={selectedModel?.brand ?? null}
              label={selectedModel?.label || thread.model || 'Choose model'}
              onPress={onOpenModelPicker}
            />
            <PickerChip
              icon="bolt"
              leading={activeAgent ? <AgentAvatar {...activeAgent} size={14} /> : undefined}
              label={missingAgent ? 'Missing Agent' : activeAgent?.name || 'No Agent'}
              warning={missingAgent}
              onPress={onOpenAgentPicker}
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
            value={draft}
            onChangeText={controller.setDraft}
            placeholder={thread.busy ? 'Queue another turn…' : 'Message milim…'}
            placeholderTextColor={palette.placeholder}
            accessibilityLabel="Message"
            multiline
            maxLength={32_000}
            onFocus={() => {
              onFocusChange(true);
              onForcedOpenChange(true);
            }}
            onBlur={() => {
              onFocusChange(false);
              onForcedOpenChange(false);
            }}
          />
          <View style={styles.composerActions}>
            <IconButton
              icon="paperclip"
              label="Add attachment"
              onPress={onOpenAttachmentMenu}
            />
            <View style={styles.composerSpacer} />
            {thread.busy ? (
              <>
                {activeRun?.capabilities.steering ? (
                  <IconButton
                    icon="bolt"
                    label="Steer next step"
                    tone="quiet"
                    disabled={busy || Boolean(controller.pendingRetry) || nothingToSend}
                    onPress={() => void steer()}
                  />
                ) : null}
                <IconButton icon="square" label="Stop generating" tone="quiet" onPress={stop} />
              </>
            ) : (
              <IconButton icon="refresh" label="Regenerate" onPress={() => void controller.command('turn.regenerate', null, thread.id, thread.revision).catch(showError)} />
            )}
            <IconButton
              icon="arrow-up"
              label={busy ? 'Sending' : thread.busy ? 'Queue message' : 'Send message'}
              tone="accent"
              disabled={busy || Boolean(controller.pendingRetry) || missingAgent || nothingToSend}
              onPress={() => void send()}
            />
          </View>
        </View>
      </Animated.View>
    </Animated.View>
  );
});
