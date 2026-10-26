import { lazy, Suspense, useEffect, useState } from "react";
import type { UserDecisionResponseV1 } from "../api.js";
import {
  askUserAnswersComplete,
  askUserAnswerText,
  PLAN_APPROVED_TEXT,
  type ChatUserDecision,
  type UserQuestionAnswer,
} from "../lib/userDecisions.js";
import { ArrowRight, Check, Pencil } from "./icons.js";

const Markdown = lazy(() =>
  import("./Markdown.js").then((mod) => ({ default: mod.Markdown })),
);

/** Sends the answering turn; false when it could not start. */
export type UserDecisionAnswerHandler = (
  response: UserDecisionResponseV1,
  text: string,
) => boolean;

function statusLabel(decision: ChatUserDecision): string {
  if (decision.status === "superseded") return "Superseded by a later message";
  if (decision.response === "approve_plan") return "Plan approved";
  if (decision.response === "revise_plan") return "Changes requested";
  return "Answered";
}

/**
 * The decision an assistant turn ended on: `ask_user` questions with
 * selectable options and free text, or an `exit_plan_mode` plan to approve.
 * Only a pending decision takes input; Rust rejects a second answer.
 */
export function UserDecisionCard({
  decision,
  disabled,
  onAnswer,
}: {
  decision: ChatUserDecision;
  disabled: boolean;
  onAnswer: UserDecisionAnswerHandler;
}) {
  const [submitted, setSubmitted] = useState(false);
  // A finished or failed send re-enables the card; a closed decision stays
  // read-only through its status.
  useEffect(() => {
    if (!disabled) setSubmitted(false);
  }, [disabled]);
  const pending = decision.status === "pending";
  const locked = disabled || submitted || !pending;
  const answer = (response: UserDecisionResponseV1, text: string) => {
    if (locked) return;
    setSubmitted(true);
    if (!onAnswer(response, text)) setSubmitted(false);
  };
  return (
    <div
      className={`user-decision-card ${decision.status}`}
      data-testid="user-decision-card"
      data-decision-kind={decision.kind}
    >
      <div className="user-decision-title">
        {decision.kind === "ask_user" ? "Questions for you" : "Plan ready for review"}
      </div>
      {decision.kind === "ask_user" ? (
        <AskUserBody decision={decision} locked={locked} onAnswer={answer} />
      ) : (
        <PlanBody decision={decision} locked={locked} onAnswer={answer} />
      )}
      {!pending && (
        <div className="user-decision-status" data-testid="user-decision-status">
          {statusLabel(decision)}
        </div>
      )}
    </div>
  );
}

function AskUserBody({
  decision,
  locked,
  onAnswer,
}: {
  decision: ChatUserDecision;
  locked: boolean;
  onAnswer: (response: UserDecisionResponseV1, text: string) => void;
}) {
  const [answers, setAnswers] = useState<UserQuestionAnswer[]>(() =>
    decision.questions.map(() => ({ selected: [], other: "" })),
  );
  const update = (index: number, next: Partial<UserQuestionAnswer>) =>
    setAnswers((current) =>
      current.map((answer, at) => (at === index ? { ...answer, ...next } : answer)),
    );
  const toggle = (index: number, label: string, multi: boolean) => {
    const selected = answers[index]?.selected ?? [];
    const chosen = selected.includes(label);
    update(index, {
      selected: multi
        ? chosen
          ? selected.filter((item) => item !== label)
          : [...selected, label]
        : chosen
          ? []
          : [label],
    });
  };
  const pending = decision.status === "pending";
  return (
    <>
      {decision.questions.map((question, index) => (
        <fieldset className="user-decision-question" key={index} disabled={locked}>
          <legend>
            <span className="user-decision-header">{question.header}</span>
            <span>{question.question}</span>
          </legend>
          <div
            className="user-decision-options"
            role={question.multi_select ? "group" : "radiogroup"}
            aria-label={question.header}
          >
            {question.options.map((option) => {
              const selected = answers[index]?.selected.includes(option.label) ?? false;
              return (
                <button
                  key={option.label}
                  type="button"
                  role={question.multi_select ? "checkbox" : "radio"}
                  aria-checked={selected}
                  className={`user-decision-option${selected ? " selected" : ""}`}
                  onClick={() => toggle(index, option.label, question.multi_select)}
                >
                  <span className="user-decision-option-label">
                    {selected && <Check size={12} />}
                    {option.label}
                  </span>
                  {option.description && (
                    <span className="user-decision-option-description">
                      {option.description}
                    </span>
                  )}
                </button>
              );
            })}
          </div>
          {pending && (
            <input
              className="user-decision-other"
              aria-label={`Other answer for ${question.header}`}
              placeholder="Other..."
              value={answers[index]?.other ?? ""}
              onChange={(event) => update(index, { other: event.target.value })}
            />
          )}
        </fieldset>
      ))}
      {pending && (
        <div className="approval-actions user-decision-actions">
          <button
            className="approval-btn approve"
            data-testid="user-decision-submit"
            type="button"
            disabled={locked || !askUserAnswersComplete(decision, answers)}
            onClick={() => onAnswer("answer", askUserAnswerText(decision, answers))}
          >
            <ArrowRight size={13} />
            <span>Send answers</span>
          </button>
        </div>
      )}
    </>
  );
}

function PlanBody({
  decision,
  locked,
  onAnswer,
}: {
  decision: ChatUserDecision;
  locked: boolean;
  onAnswer: (response: UserDecisionResponseV1, text: string) => void;
}) {
  const [revising, setRevising] = useState(false);
  const [feedback, setFeedback] = useState("");
  const plan = decision.plan ?? "";
  return (
    <>
      <div className="user-decision-plan">
        <Suspense fallback={<pre>{plan}</pre>}>
          <Markdown content={plan} highlight collapseArtifacts={false} />
        </Suspense>
      </div>
      {decision.status === "pending" && (
        revising ? (
          <div className="user-decision-feedback">
            <textarea
              className="user-decision-other"
              aria-label="What should change in the plan?"
              placeholder="What should change?"
              rows={3}
              value={feedback}
              disabled={locked}
              autoFocus
              onChange={(event) => setFeedback(event.target.value)}
            />
            <div className="approval-actions user-decision-actions">
              <button
                className="approval-btn"
                type="button"
                disabled={locked}
                onClick={() => setRevising(false)}
              >
                <span>Cancel</span>
              </button>
              <button
                className="approval-btn approve"
                data-testid="user-decision-send-feedback"
                type="button"
                disabled={locked || !feedback.trim()}
                onClick={() => onAnswer("revise_plan", feedback)}
              >
                <ArrowRight size={13} />
                <span>Send feedback</span>
              </button>
            </div>
          </div>
        ) : (
          <div className="approval-actions user-decision-actions">
            <button
              className="approval-btn approve"
              data-testid="user-decision-approve-plan"
              type="button"
              disabled={locked}
              onClick={() => onAnswer("approve_plan", PLAN_APPROVED_TEXT)}
            >
              <Check size={13} />
              <span>Approve and implement</span>
            </button>
            <button
              className="approval-btn"
              data-testid="user-decision-keep-planning"
              type="button"
              disabled={locked}
              onClick={() => setRevising(true)}
            >
              <Pencil size={13} />
              <span>Keep planning</span>
            </button>
          </div>
        )
      )}
    </>
  );
}
