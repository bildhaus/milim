import {useEffect, useRef, useState} from 'react';
import {ActivityIndicator, ScrollView, View} from 'react-native';
import {type RunEventPageV1, type RunEventV1, type RunInspectionV1} from '../control/generated-v1';
import {type ActivityStatus, type ProjectedActivityGroup, type ProjectedActivityIcon, type ProjectedActivityRow} from '../control/replica';
import {useMilimController} from '../controller/useMilimController';
import {type MobilePalette} from '../theme';
import {MilimIcon, type MilimIconName} from '../ui/MilimIcon';
import {useAppTheme, Text} from '../ui/appTheme';
import {MotionPressable} from '../ui/motion';

export function activityStatusLabel(status: ActivityStatus): string {
  switch (status) {
    case 'running': return 'In progress';
    case 'completed': return 'Completed';
    case 'warning': return 'Needs attention';
    case 'failed': return 'Failed';
    case 'approval': return 'Approval needed';
  }
}

export function activityStatusColor(status: ActivityStatus, palette: MobilePalette): string {
  switch (status) {
    case 'running': return palette.success;
    case 'completed': return palette.secondary;
    case 'warning':
    case 'approval': return palette.warning;
    case 'failed': return palette.danger;
  }
}

export function activityIconName(icon: ProjectedActivityIcon, status: ActivityStatus): MilimIconName {
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

export function ActivityRow({row}: {row: ProjectedActivityRow}) {
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

export function RunDetailSection({label, value}: {label: string; value: unknown}) {
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

export function runEventGroups(events: RunEventV1[], inspection: RunInspectionV1) {
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

export function MobileRunDetails({
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

export function ActivityGroup({
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
