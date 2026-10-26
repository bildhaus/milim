//! `ask_user` and `exit_plan_mode`: hand the user a structured decision and
//! end the run. The agent loop sees `user_decision_notice`, records every
//! result of the step, and stops; the answer is the thread's next turn.
//!
//! They are offered only to interactive canonical runs, never to Workers,
//! scheduled or linked-thread turns, or HTTP runs.

use milim_control_contract::{
    UserDecisionV1, UserQuestionV1, USER_DECISION_MAX_DESCRIPTION_CHARS,
    USER_DECISION_MAX_HEADER_CHARS, USER_DECISION_MAX_LABEL_CHARS, USER_DECISION_MAX_OPTIONS,
    USER_DECISION_MAX_PLAN_CHARS, USER_DECISION_MAX_QUESTIONS, USER_DECISION_MAX_QUESTION_CHARS,
    USER_DECISION_MIN_OPTIONS,
};

use super::*;

pub(super) const ASK_USER_TOOL: &str = "ask_user";
pub(super) const EXIT_PLAN_MODE_TOOL: &str = "exit_plan_mode";
const WAITING_FOR_USER: &str =
    "Waiting for the user's answer; it will arrive as the next message. End your turn now.";

/// Add the decision tools an interactive run may call. `exit_plan_mode`
/// exists only while plan mode is on. A custom tool mode gets the ones it
/// names; `none` gets neither.
pub(super) fn register_user_decision_tools(
    reg: &mut ToolRegistry,
    tool_mode: &str,
    enabled_tools: &[String],
    plan_mode: bool,
) {
    let normalized = milim_agents::normalize_tool_mode(tool_mode, enabled_tools);
    let wanted = |name: &str| match normalized.as_str() {
        "none" => false,
        "custom" => enabled_tools.iter().any(|tool| tool == name),
        _ => true,
    };
    if wanted(ASK_USER_TOOL) {
        reg.register(Arc::new(AskUserTool));
    }
    if plan_mode && wanted(EXIT_PLAN_MODE_TOOL) {
        reg.register(Arc::new(ExitPlanModeTool));
    }
}

fn waiting_result(decision: UserDecisionV1) -> Value {
    json!({
        "status": "waiting_for_user",
        "decision_id": decision.id,
        "user_decision_notice": decision,
    })
}

fn waiting_text(result: &Value) -> Option<String> {
    (result.get("status").and_then(Value::as_str) == Some("waiting_for_user"))
        .then(|| WAITING_FOR_USER.to_string())
}

pub(super) struct AskUserTool;

#[derive(Deserialize)]
struct AskUserArgs {
    questions: Vec<UserQuestionV1>,
}

#[async_trait]
impl Tool for AskUserTool {
    fn name(&self) -> &str {
        ASK_USER_TOOL
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    fn description(&self) -> &str {
        "Ask the user 1-4 multiple-choice questions when a genuine ambiguity would make a wrong guess costly. Each question offers 2-4 options; the user can also answer in their own words. Your turn ends after this call and the answers arrive as the next user message, so call it last and only once."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": USER_DECISION_MAX_QUESTIONS,
                    "items": {
                        "type": "object",
                        "properties": {
                            "question": {
                                "type": "string",
                                "maxLength": USER_DECISION_MAX_QUESTION_CHARS,
                                "description": "The complete question, ending with a question mark."
                            },
                            "header": {
                                "type": "string",
                                "maxLength": USER_DECISION_MAX_HEADER_CHARS,
                                "description": "A very short label for the question, such as `Database`."
                            },
                            "options": {
                                "type": "array",
                                "minItems": USER_DECISION_MIN_OPTIONS,
                                "maxItems": USER_DECISION_MAX_OPTIONS,
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "label": { "type": "string", "maxLength": USER_DECISION_MAX_LABEL_CHARS },
                                        "description": {
                                            "type": "string",
                                            "maxLength": USER_DECISION_MAX_DESCRIPTION_CHARS,
                                            "description": "What choosing this option means, including its trade-off."
                                        }
                                    },
                                    "required": ["label", "description"],
                                    "additionalProperties": false
                                }
                            },
                            "multi_select": {
                                "type": "boolean",
                                "description": "Let the user choose more than one option."
                            }
                        },
                        "required": ["question", "header", "options"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["questions"],
            "additionalProperties": false
        })
    }

    fn model_text(&self, result: &Value) -> Option<String> {
        waiting_text(result)
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: AskUserArgs = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid ask_user arguments: {error}"))
        })?;
        let decision = UserDecisionV1::ask_user(uuid::Uuid::new_v4().to_string(), args.questions)
            .map_err(Error::InvalidRequest)?;
        Ok(waiting_result(decision))
    }
}

pub(super) struct ExitPlanModeTool;

#[derive(Deserialize)]
struct ExitPlanModeArgs {
    plan: String,
}

#[async_trait]
impl Tool for ExitPlanModeTool {
    fn name(&self) -> &str {
        EXIT_PLAN_MODE_TOOL
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    fn description(&self) -> &str {
        "Present your implementation plan for the user's approval and leave plan mode once they approve. Call it when the plan is ready, with the complete plan as Markdown: the files to change, the approach, and how to verify it. Your turn ends after this call; the user's approval or feedback arrives as the next message."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "plan": {
                    "type": "string",
                    "maxLength": USER_DECISION_MAX_PLAN_CHARS,
                    "description": "The implementation plan as Markdown."
                }
            },
            "required": ["plan"],
            "additionalProperties": false
        })
    }

    fn model_text(&self, result: &Value) -> Option<String> {
        waiting_text(result)
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: ExitPlanModeArgs = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid exit_plan_mode arguments: {error}"))
        })?;
        let decision = UserDecisionV1::exit_plan_mode(uuid::Uuid::new_v4().to_string(), &args.plan)
            .map_err(Error::InvalidRequest)?;
        Ok(waiting_result(decision))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(reg: &ToolRegistry) -> Vec<String> {
        let mut names = reg.names();
        names.sort();
        names
    }

    #[test]
    fn decision_tools_follow_tool_mode_and_plan_mode() {
        let mut reg = ToolRegistry::new();
        register_user_decision_tools(&mut reg, "all", &[], false);
        assert_eq!(names(&reg), ["ask_user"]);

        let mut reg = ToolRegistry::new();
        register_user_decision_tools(&mut reg, "all", &[], true);
        assert_eq!(names(&reg), ["ask_user", "exit_plan_mode"]);

        let mut reg = ToolRegistry::new();
        register_user_decision_tools(&mut reg, "none", &[], true);
        assert!(reg.is_empty());

        let mut reg = ToolRegistry::new();
        register_user_decision_tools(&mut reg, "custom", &["exit_plan_mode".to_string()], true);
        assert_eq!(names(&reg), ["exit_plan_mode"]);
    }

    #[tokio::test]
    async fn decision_tools_return_a_notice_and_a_short_model_text() {
        let tool = AskUserTool;
        let result = tool
            .invoke(json!({
                "questions": [{
                    "question": "Which store?",
                    "header": "Store",
                    "options": [
                        { "label": "SQLite", "description": "Embedded" },
                        { "label": "Postgres", "description": "Server" }
                    ]
                }]
            }))
            .await
            .unwrap();
        let notice = &result["user_decision_notice"];
        assert_eq!(notice["kind"], "ask_user");
        assert_eq!(notice["id"], result["decision_id"]);
        assert!(notice["fallback_text"]
            .as_str()
            .unwrap()
            .contains("**Store**: Which store?"));
        assert!(tool.model_text(&result).unwrap().starts_with("Waiting"));

        let error = tool
            .invoke(json!({ "questions": [{ "question": "Q?", "header": "H", "options": [{ "label": "only", "description": "" }] }] }))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("2 to 4"), "{error}");

        let plan = ExitPlanModeTool
            .invoke(json!({ "plan": "1. Change `a.rs`" }))
            .await
            .unwrap();
        assert_eq!(plan["user_decision_notice"]["plan"], "1. Change `a.rs`");
        assert!(ExitPlanModeTool
            .invoke(json!({ "plan": " " }))
            .await
            .is_err());
    }
}
