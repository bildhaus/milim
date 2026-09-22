import {useMemo} from 'react';
import {ScrollView, View} from 'react-native';
import {newCommandId} from '../control/client';
import {type PendingApprovalV1, type QueuedTurnV1} from '../control/types';
import {useHotState, type HotStore} from '../controller/hotStore';
import type {MilimController} from '../controller/useMilimController';
import {useAppTheme, Text} from '../ui/appTheme';
import {Button, IconButton, Empty} from '../ui/controls';
import {showError} from '../ui/dialogs';
import {ApprovalCard} from '../transcript/ApprovalCard';

export function AttentionScreen({
  approvals,
  queuedTurns,
  hot,
  execute,
  onBack,
}: {
  approvals: PendingApprovalV1[];
  queuedTurns: QueuedTurnV1[];
  hot: HotStore;
  execute: MilimController['execute'];
  onBack: () => void;
}) {
  const {styles} = useAppTheme();
  const timelineItems = useHotState(hot, state => state.timeline?.items);
  const proposals = useMemo(
    () => (timelineItems ?? []).filter(item => item.type.includes('worker') && JSON.stringify(item.data).includes('proposed')),
    [timelineItems],
  );
  return (
    <ScrollView style={styles.screen} contentContainerStyle={styles.list}>
      <View style={styles.pageHeadingRow}>
        <IconButton icon="arrow-left" label="Back to chat" onPress={onBack} />
        <View style={styles.pageHeadingCopy}>
          <Text style={styles.eyebrow}>REVIEW</Text>
          <Text style={styles.screenTitle} accessibilityRole="header">Attention</Text>
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
