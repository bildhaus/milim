import {useState} from 'react';
import {ScrollView, TextInput, View} from 'react-native';
import {newCommandId} from '../control/client';
import {type PendingApprovalV1} from '../control/types';
import {useMilimController} from '../controller/useMilimController';
import {useAppTheme, Text} from '../ui/appTheme';
import {Button} from '../ui/controls';
import {showError} from '../ui/dialogs';
import {haptics} from '../ui/haptics';

export function ApprovalCard({approval, execute, inline = false}: {approval: PendingApprovalV1; execute: ReturnType<typeof useMilimController>['execute']; inline?: boolean}) {
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
        <Button label="Deny" icon="x" tone="danger" onPress={() => {
          haptics.warning();
          void resolve('deny').catch(showError);
        }} />
        <Button label="Approve" icon="check" disabled={!supported} onPress={() => {
          haptics.success();
          void resolve('approve').catch(showError);
        }} />
      </View>
    </View>
  );
}
