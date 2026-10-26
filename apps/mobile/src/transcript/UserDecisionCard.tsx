import {useState} from 'react';
import {TextInput, View} from 'react-native';
import {newCommandId} from '../control/client';
import type {UserDecisionResponseV1, UserQuestionV1} from '../control/generated-v1';
import type {ProjectedUserDecision} from '../control/replica';
import {
  askUserAnswerReady,
  askUserAnswerText,
  decisionAnswerPayload,
  decisionStatusLabel,
  emptyAnswerDrafts,
  PLAN_APPROVED_TEXT,
  toggleOption,
  type QuestionAnswerDraft,
} from '../control/userDecisions';
import type {useMilimController} from '../controller/useMilimController';
import {MilimIcon} from '../ui/MilimIcon';
import {useAppTheme, Text} from '../ui/appTheme';
import {Button} from '../ui/controls';
import {haptics} from '../ui/haptics';
import {MotionPressable} from '../ui/motion';
import {MessageMarkdown} from './MessageMarkdown';

type Execute = ReturnType<typeof useMilimController>['execute'];
type MarkdownStyles = ReturnType<typeof useAppTheme>['markdownStyles'];

// A decision a run ended on. Answering sends an ordinary `turn.send` that
// names the decision; typing in the composer instead supersedes it host-side.
export function UserDecisionCard({
  item,
  execute,
  markdownStyles,
}: {
  item: ProjectedUserDecision;
  execute: Execute;
  markdownStyles: MarkdownStyles;
}) {
  const {palette, styles} = useAppTheme();
  const {decision} = item;
  const pending = item.state === 'pending';
  const [drafts, setDrafts] = useState<QuestionAnswerDraft[]>(() => emptyAnswerDrafts(decision));
  const [revising, setRevising] = useState(false);
  const [feedback, setFeedback] = useState('');
  const [inFlight, setInFlight] = useState(false);
  // Accepted but not yet reflected by a timeline sync.
  const [sent, setSent] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const locked = !pending || inFlight || sent;

  const answer = async (response: UserDecisionResponseV1, text: string) => {
    if (locked) return;
    haptics.send();
    setInFlight(true);
    setNotice(null);
    try {
      await execute({
        command_id: newCommandId(),
        kind: 'turn.send',
        thread_id: item.threadId,
        payload: decisionAnswerPayload(decision, response, text),
      });
      setSent(true);
    } catch (error) {
      // A stale answer comes back as `conflict`; the next sync settles the card.
      setNotice(error instanceof Error ? error.message : String(error));
    } finally {
      setInFlight(false);
    }
  };

  const statusColor = item.state === 'answered'
    ? palette.success
    : item.state === 'superseded'
      ? palette.muted
      : palette.warning;
  const plan = decision.kind === 'exit_plan_mode';

  return (
    <View style={[styles.attentionCard, styles.inlineApprovalCard, !pending && styles.decisionCardSettled]}>
      <Text style={styles.messageRole}>{plan ? 'PLAN' : 'QUESTION'}</Text>
      <Text style={styles.attentionTitle}>{plan ? 'Review the plan' : 'Your input is needed'}</Text>
      {plan ? (
        <MessageMarkdown content={decision.plan || decision.fallback_text || 'No plan was provided.'} style={markdownStyles} />
      ) : (
        decision.questions.map((question, index) => (
          <QuestionBlock
            key={`${index}-${question.header}`}
            question={question}
            draft={drafts[index]}
            disabled={locked}
            readOnly={!pending}
            onChange={next => setDrafts(current => current.map((entry, at) => at === index ? next : entry))}
          />
        ))
      )}
      <Text style={[styles.activitySummaryStatus, {color: statusColor}]}>
        {sent && pending ? 'Sending answer…' : decisionStatusLabel(item.state, item.response)}
      </Text>
      {notice ? <Text style={styles.missing}>{notice}</Text> : null}
      {pending && plan && revising ? (
        <TextInput
          style={styles.input}
          value={feedback}
          onChangeText={setFeedback}
          editable={!locked}
          multiline
          placeholder="What should change in the plan?"
          placeholderTextColor={palette.placeholder}
        />
      ) : null}
      {pending ? (
        <View style={styles.actionRow}>
          {plan ? (
            revising ? (
              <>
                <Button label="Cancel" tone="quiet" disabled={locked} onPress={() => setRevising(false)} />
                <Button
                  label="Send feedback"
                  icon="arrow-up"
                  disabled={locked || !feedback.trim()}
                  onPress={() => void answer('revise_plan', feedback.trim())}
                />
              </>
            ) : (
              <>
                <Button label="Keep planning" icon="pencil" tone="quiet" disabled={locked} onPress={() => setRevising(true)} />
                <Button
                  label="Approve and implement"
                  icon="check"
                  disabled={locked}
                  onPress={() => void answer('approve_plan', PLAN_APPROVED_TEXT)}
                />
              </>
            )
          ) : (
            <Button
              label="Submit"
              icon="arrow-up"
              disabled={locked || !askUserAnswerReady(drafts)}
              onPress={() => void answer('answer', askUserAnswerText(decision.questions, drafts))}
            />
          )}
        </View>
      ) : null}
    </View>
  );
}

function QuestionBlock({
  question,
  draft,
  disabled,
  readOnly,
  onChange,
}: {
  question: UserQuestionV1;
  draft: QuestionAnswerDraft | undefined;
  disabled: boolean;
  readOnly: boolean;
  onChange: (next: QuestionAnswerDraft) => void;
}) {
  const {palette, styles} = useAppTheme();
  const current = draft ?? {selected: [], other: ''};
  return (
    <View style={styles.decisionQuestion}>
      <Text style={styles.reasoningEffortLabel}>{question.header}</Text>
      <Text style={styles.decisionQuestionText}>{question.question}</Text>
      {question.options.map(option => {
        const selected = current.selected.includes(option.label);
        return (
          <MotionPressable
            key={option.label}
            style={[styles.pickerRow, styles.decisionOption, selected && styles.pickerRowSelected, disabled && !selected && styles.disabled]}
            disabled={disabled}
            accessibilityRole={question.multi_select ? 'checkbox' : 'radio'}
            accessibilityState={{checked: selected, disabled}}
            accessibilityLabel={option.description ? `${option.label}. ${option.description}` : option.label}
            onPress={() => {
              haptics.selection();
              onChange(toggleOption(current, question, option.label));
            }}>
            <View style={styles.pickerRowIcon}>
              {selected ? <MilimIcon name="check" size={15} color={palette.accent} /> : null}
            </View>
            <View style={styles.pickerRowBody}>
              <Text style={styles.pickerRowTitle}>{option.label}</Text>
              {option.description ? <Text style={styles.pickerRowDescription}>{option.description}</Text> : null}
            </View>
          </MotionPressable>
        );
      })}
      {!readOnly ? (
        <TextInput
          style={styles.input}
          editable={!disabled}
          value={current.other}
          onChangeText={other => onChange({...current, other})}
          placeholder="Other"
          placeholderTextColor={palette.placeholder}
        />
      ) : null}
    </View>
  );
}
