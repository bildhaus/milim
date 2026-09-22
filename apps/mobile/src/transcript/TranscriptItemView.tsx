import React, {useState} from 'react';
import {Pressable, View} from 'react-native';
import {type ProjectedTranscriptItem} from '../control/replica';
import {useMilimController} from '../controller/useMilimController';
import {transcriptModelLabel} from '../modelPicker';
import {MilimIcon} from '../ui/MilimIcon';
import {createMarkdownStyles} from '../ui/styles';
import {useAppTheme, Text} from '../ui/appTheme';
import {MotionPressable} from '../ui/motion';
import {haptics} from '../ui/haptics';
import {MessageMarkdown} from './MessageMarkdown';
import type {MessageActionTarget} from './MessageActionsSheet';
import {activityStatusLabel, activityStatusColor, activityIconName, ActivityGroup} from './activity';
import {ApprovalCard} from './ApprovalCard';

export const TranscriptItemView = React.memo(function MemoizedTranscriptItem({
  item,
  markdownStyles,
  execute,
  runDetailsEnabled,
  loadRunDetails,
  loadMoreRunEvents,
  onMessageActions,
}: {
  item: ProjectedTranscriptItem;
  markdownStyles: ReturnType<typeof createMarkdownStyles>;
  execute: ReturnType<typeof useMilimController>['execute'];
  runDetailsEnabled: boolean;
  loadRunDetails: ReturnType<typeof useMilimController>['loadRunDetails'];
  loadMoreRunEvents: ReturnType<typeof useMilimController>['loadMoreRunEvents'];
  onMessageActions: (target: MessageActionTarget) => void;
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
  const openActions = () => {
    if (!item.content) return;
    haptics.selection();
    onMessageActions({role: item.role, content: item.content});
  };
  return (
    <Pressable
      style={({pressed}) => [styles.message, item.role === 'user' ? styles.userMessage : styles.assistantMessage, pressed && styles.messagePressed]}
      onLongPress={openActions}
      delayLongPress={350}
      accessibilityActions={[{name: 'longpress', label: 'Message actions'}]}
      onAccessibilityAction={event => {
        if (event.nativeEvent.actionName === 'longpress') openActions();
      }}>
      {item.steering ? (
        <Text style={styles.messageRole}>{item.steeringPending ? 'STEER · PENDING' : 'STEER'}</Text>
      ) : null}
      {item.mailboxLabel ? <Text style={styles.messageRole}>{item.mailboxLabel}</Text> : null}
      {item.role === 'system' ? <Text style={styles.messageRole}>SYSTEM</Text> : null}
      {item.reasoning ? <ReasoningBlock text={item.reasoning} /> : null}
      <MessageMarkdown content={item.content || '…'} style={markdownStyles} />
    </Pressable>
  );
});

export function ReasoningBlock({text}: {text: string}) {
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
