use std::collections::HashMap;

use milim_core::api::openai::{
    ChatMessage, Content, ContentPart, ImageUrl, ReasoningEffort, Tool, ToolFunction,
};
use milim_inference::{gemini::GeminiBackend, CompletionRequest, ModelService, SamplingParams};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;

#[derive(Debug)]
struct CapturedRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: serde_json::Value,
}

async fn spawn_once(
    response_body: &'static str,
    content_type: &'static str,
) -> (String, oneshot::Receiver<CapturedRequest>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = oneshot::channel();

    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let mut buf = [0u8; 1024];

        loop {
            let n = socket.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&buf[..n]);
            if request_complete(&bytes) {
                break;
            }
        }

        let captured = parse_request(&bytes);
        let _ = tx.send(captured);

        let resp = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        socket.write_all(resp.as_bytes()).await.unwrap();
    });

    (format!("http://{addr}/v1beta"), rx)
}

fn request_complete(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return false;
    };
    let len = head
        .lines()
        .find_map(|l| {
            l.strip_prefix("Content-Length:")
                .or_else(|| l.strip_prefix("content-length:"))
        })
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    body.len() >= len
}

fn parse_request(bytes: &[u8]) -> CapturedRequest {
    let text = String::from_utf8_lossy(bytes);
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    let mut lines = head.lines();
    let first = lines.next().unwrap();
    let mut first_parts = first.split_whitespace();
    let method = first_parts.next().unwrap().to_string();
    let path = first_parts.next().unwrap().to_string();
    let headers = lines
        .filter_map(|line| {
            let (k, v) = line.split_once(':')?;
            Some((k.to_ascii_lowercase(), v.trim().to_string()))
        })
        .collect();
    let body = if body.trim().is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_str(body).unwrap()
    };

    CapturedRequest {
        method,
        path,
        headers,
        body,
    }
}

fn basic_req(model: &str) -> CompletionRequest {
    CompletionRequest {
        model: model.to_string(),
        messages: vec![
            ChatMessage::text("system", "Be concise."),
            ChatMessage::text("user", "Hello"),
        ],
        tools: vec![],
        tool_choice: None,
        response_format: None,
        prompt: None,
        suffix: None,
        sampling: SamplingParams {
            max_tokens: Some(64),
            temperature: Some(0.2),
            stop: vec!["END".to_string()],
            ..Default::default()
        },
        reasoning_effort: None,
    }
}

#[tokio::test]
async fn lists_gemini_models_with_required_headers() {
    let body = r#"{"models":[{"name":"models/gemini-2.5-flash","displayName":"Gemini 2.5 Flash","inputTokenLimit":1048576,"outputTokenLimit":8192}]}"#;
    let (base, captured) = spawn_once(body, "application/json").await;
    let backend = GeminiBackend::new("gemini", base, Some("AIza-test".to_string()));

    let models = backend.list_models().await.unwrap();
    let req = captured.await.unwrap();

    assert_eq!(req.method, "GET");
    assert_eq!(req.path, "/v1beta/models");
    assert_eq!(
        req.headers.get("x-goog-api-key").map(String::as_str),
        Some("AIza-test")
    );
    assert_eq!(models[0].id, "gemini-2.5-flash");
    assert_eq!(models[0].owned_by, "gemini");
    assert_eq!(models[0].context_length, Some(1_048_576));
    assert_eq!(models[0].max_prompt_tokens, Some(1_048_576));
    assert_eq!(models[0].max_completion_tokens, Some(8192));
}

#[tokio::test]
async fn streams_text_and_builds_generate_content_body() {
    let sse = concat!(
        r#"data: {"candidates":[{"content":{"parts":[{"text":"Hello"}],"role":"model"},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":7,"candidatesTokenCount":3,"totalTokenCount":10}}"#,
        "\n\n"
    );
    let (base, captured) = spawn_once(sse, "text/event-stream").await;
    let backend = GeminiBackend::new("gemini", base, Some("AIza-test".to_string()));

    let mut input = basic_req("gemini-2.5-flash");
    input.reasoning_effort = Some(ReasoningEffort::Medium);
    let out = backend.complete(input).await.unwrap();
    let req = captured.await.unwrap();

    assert_eq!(req.method, "POST");
    assert_eq!(
        req.path,
        "/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
    );
    assert_eq!(
        req.body["systemInstruction"]["parts"][0]["text"],
        "Be concise."
    );
    assert_eq!(req.body["contents"][0]["role"], "user");
    assert_eq!(req.body["contents"][0]["parts"][0]["text"], "Hello");
    assert_eq!(req.body["generationConfig"]["maxOutputTokens"], 64);
    let temp = req.body["generationConfig"]["temperature"]
        .as_f64()
        .unwrap();
    assert!((temp - 0.2).abs() < 0.000_001, "temperature was {temp}");
    assert_eq!(req.body["generationConfig"]["stopSequences"][0], "END");
    assert_eq!(
        req.body["generationConfig"]["thinkingConfig"]["thinkingBudget"],
        4096
    );

    assert_eq!(out.message.text_content(), "Hello");
    assert_eq!(out.finish_reason, "stop");
    assert_eq!(out.usage.prompt_tokens, 7);
    assert_eq!(out.usage.completion_tokens, 3);
    assert_eq!(out.usage.total_tokens, 10);
}

#[tokio::test]
async fn sends_uploaded_image_bytes_as_gemini_inline_data() {
    let sse = concat!(
        r#"data: {"candidates":[{"content":{"parts":[{"text":"red, green, blue, white"}],"role":"model"},"finishReason":"STOP"}]}"#,
        "\n\n"
    );
    let (base, captured) = spawn_once(sse, "text/event-stream").await;
    let backend = GeminiBackend::new("gemini", base, Some("AIza-test".to_string()));
    let png = include_str!("fixtures/geometric-png.base64").trim();
    let mut req = basic_req("gemini-2.5-flash");
    req.messages = vec![ChatMessage {
        role: "user".to_string(),
        content: Some(Content::Parts(vec![
            ContentPart::Text {
                text: "What colors are present?".to_string(),
            },
            ContentPart::ImageUrl {
                image_url: ImageUrl {
                    url: format!("data:image/png;base64,{png}"),
                    detail: None,
                },
            },
        ])),
        name: None,
        tool_calls: None,
        tool_call_id: None,
        reasoning_content: None,
        provider_state: None,
    }];

    backend.complete(req).await.unwrap();
    let sent = captured.await.unwrap();
    assert_eq!(
        sent.body["contents"][0]["parts"][1]["inline_data"]["mime_type"],
        "image/png"
    );
    assert_eq!(
        sent.body["contents"][0]["parts"][1]["inline_data"]["data"],
        png
    );
}

#[tokio::test]
async fn streams_function_call_as_openai_tool_call() {
    let sse = concat!(
        r#"data: {"candidates":[{"content":{"parts":[{"functionCall":{"name":"get_weather","args":{"location":"Paris"}}}],"role":"model"},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":12,"candidatesTokenCount":4,"totalTokenCount":16}}"#,
        "\n\n"
    );
    let (base, captured) = spawn_once(sse, "text/event-stream").await;
    let backend = GeminiBackend::new("gemini", base, Some("AIza-test".to_string()));
    let mut req = basic_req("models/gemini-2.5-flash");
    req.tools = vec![
        Tool {
            kind: "function".to_string(),
            function: ToolFunction {
                name: "get_weather".to_string(),
                description: Some("Get weather.".to_string()),
                parameters: Some(serde_json::json!({"type":"object"})),
            },
        },
        Tool {
            kind: "function".to_string(),
            function: ToolFunction {
                name: "no_description".to_string(),
                description: None,
                parameters: None,
            },
        },
    ];

    let out = backend.complete(req).await.unwrap();
    let sent = captured.await.unwrap();
    let calls = out.message.tool_calls.unwrap();

    assert_eq!(
        sent.path,
        "/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
    );
    assert_eq!(
        sent.body["tools"][0]["functionDeclarations"][0]["name"],
        "get_weather"
    );
    assert_eq!(
        sent.body["tools"][0]["functionDeclarations"][0]["description"],
        "Get weather."
    );
    assert_eq!(
        sent.body["tools"][0]["functionDeclarations"][0]["parameters"]["type"],
        "object"
    );
    assert!(sent.body["tools"][0]["functionDeclarations"][1]
        .get("description")
        .is_none());
    assert_eq!(
        sent.body["tools"][0]["functionDeclarations"][1]["parameters"]["type"],
        "object"
    );
    assert_eq!(out.finish_reason, "tool_calls");
    assert!(calls[0]
        .id
        .as_deref()
        .is_some_and(|id| id.starts_with("gemini_call_")));
    assert_eq!(calls[0].function.name, "get_weather");
    assert_eq!(calls[0].function.arguments, r#"{"location":"Paris"}"#);
}

#[tokio::test]
async fn later_system_messages_become_in_place_user_reminders() {
    let sse = concat!(
        r#"data: {"candidates":[{"content":{"parts":[{"text":"ok"}],"role":"model"},"finishReason":"MAX_TOKENS"}],"usageMetadata":{"promptTokenCount":900,"cachedContentTokenCount":600,"candidatesTokenCount":64,"totalTokenCount":964}}"#,
        "\n\n"
    );
    let (base, captured) = spawn_once(sse, "text/event-stream").await;
    let backend = GeminiBackend::new("gemini", base, Some("AIza-test".to_string()));
    let mut req = basic_req("gemini-2.5-flash");
    req.messages = vec![
        ChatMessage::text("system", "Base prompt."),
        ChatMessage::text("system", "Project rules."),
        ChatMessage::text("user", "Fix the bug."),
        ChatMessage::text("assistant", "Looking."),
        ChatMessage::text("system", "Plan mode is off."),
        ChatMessage::text("user", "Go ahead."),
    ];

    let out = backend.complete(req).await.unwrap();
    let sent = captured.await.unwrap();

    assert_eq!(
        sent.body["systemInstruction"],
        serde_json::json!({ "parts": [{ "text": "Base prompt." }, { "text": "Project rules." }] })
    );
    let contents = sent.body["contents"].as_array().unwrap();
    assert_eq!(contents.len(), 3);
    assert_eq!(contents[1]["role"], "model");
    assert_eq!(
        contents[2],
        serde_json::json!({
            "role": "user",
            "parts": [
                { "text": "<system-reminder>\nPlan mode is off.\n</system-reminder>" },
                { "text": "Go ahead." }
            ]
        })
    );
    assert_eq!(out.finish_reason, "length");
    assert_eq!(out.usage.prompt_tokens, 900);
    assert_eq!(out.usage.cache_read_tokens, Some(600));
}

#[tokio::test]
async fn safety_stops_report_content_filter() {
    let sse = concat!(
        r#"data: {"candidates":[{"finishReason":"SAFETY"}]}"#,
        "\n\n"
    );
    let (base, _captured) = spawn_once(sse, "text/event-stream").await;
    let backend = GeminiBackend::new("gemini", base, None);
    let out = backend
        .complete(basic_req("gemini-2.5-flash"))
        .await
        .unwrap();
    assert_eq!(out.finish_reason, "content_filter");
}

#[tokio::test]
async fn mid_stream_error_payloads_are_typed_upstream_errors() {
    let sse = concat!(
        r#"data: {"candidates":[{"content":{"parts":[{"text":"partial"}],"role":"model"}}]}"#,
        "\n\n",
        r#"data: {"error":{"code":503,"message":"The model is overloaded. Please try again later.","status":"UNAVAILABLE"}}"#,
        "\n\n"
    );
    let (base, _captured) = spawn_once(sse, "text/event-stream").await;
    let backend = GeminiBackend::new("gemini", base, None);

    let err = backend
        .complete(basic_req("gemini-2.5-flash"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("-> 503 UNAVAILABLE"), "{err}");
    assert!(
        milim_core::provider_error::retry_hint(&err)
            .unwrap()
            .retryable
    );
}

#[tokio::test]
async fn counts_thinking_tokens_as_output() {
    let sse = concat!(
        r#"data: {"candidates":[{"content":{"parts":[{"text":"Hi"}],"role":"model"},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":2,"thoughtsTokenCount":300,"totalTokenCount":312}}"#,
        "\n\n"
    );
    let (base, _captured) = spawn_once(sse, "text/event-stream").await;
    let backend = GeminiBackend::new("gemini", base, None);
    let out = backend.complete(basic_req("gemini-2.5-pro")).await.unwrap();
    assert_eq!(out.usage.prompt_tokens, 10);
    assert_eq!(out.usage.completion_tokens, 302);
    assert_eq!(out.usage.total_tokens, 312);
}

#[tokio::test]
async fn returns_thought_signatures_as_provider_state() {
    let sse = concat!(
        r#"data: {"candidates":[{"content":{"parts":[{"functionCall":{"name":"get_weather","args":{"location":"Paris"}},"thoughtSignature":"c2lnLTE="}],"role":"model"}}]}"#,
        "\n\n",
        r#"data: {"candidates":[{"content":{"parts":[{"text":"","thoughtSignature":"c2lnLTI="}],"role":"model"},"finishReason":"STOP"}]}"#,
        "\n\n"
    );
    let (base, _captured) = spawn_once(sse, "text/event-stream").await;
    let backend = GeminiBackend::new("gemini", base, None);
    let out = backend
        .complete(basic_req("gemini-3-pro-preview"))
        .await
        .unwrap();
    let call_id = out.message.tool_calls.as_ref().unwrap()[0]
        .id
        .clone()
        .unwrap();
    assert_eq!(
        out.message.provider_state,
        Some(serde_json::json!({"gemini": {
            "signatures": { call_id.clone(): "c2lnLTE=" },
            "text_signature": "c2lnLTI="
        }}))
    );

    // A different backend instance (as after a restart) signs the replay
    // from the saved provider state.
    let sse = concat!(
        r#"data: {"candidates":[{"content":{"parts":[{"text":"Sunny."}],"role":"model"},"finishReason":"STOP"}]}"#,
        "\n\n"
    );
    let (base, captured) = spawn_once(sse, "text/event-stream").await;
    let restarted = GeminiBackend::new("gemini", base, None);
    let mut req = basic_req("gemini-3-pro-preview");
    req.messages.push(out.message);
    req.messages.push(ChatMessage {
        role: "tool".to_string(),
        content: Some(Content::Text(r#"{"temp":21}"#.to_string())),
        name: Some("get_weather".to_string()),
        tool_calls: None,
        tool_call_id: Some(call_id),
        reasoning_content: None,
        provider_state: None,
    });
    restarted.complete(req).await.unwrap();
    let sent = captured.await.unwrap();
    let model_parts = &sent.body["contents"][1]["parts"];
    assert_eq!(model_parts[0]["functionCall"]["name"], "get_weather");
    assert_eq!(model_parts[0]["thoughtSignature"], "c2lnLTE=");
}

#[tokio::test]
async fn stream_without_finish_reason_is_a_retryable_cutoff() {
    let sse = concat!(
        r#"data: {"candidates":[{"content":{"parts":[{"text":"partial"}],"role":"model"}}]}"#,
        "\n\n"
    );
    let (base, _captured) = spawn_once(sse, "text/event-stream").await;
    let backend = GeminiBackend::new("gemini", base, None);
    let err = backend
        .complete(basic_req("gemini-2.5-flash"))
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("stream ended before a completion event"),
        "{err}"
    );
}
