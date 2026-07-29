use super::*;

/// `GET /v1/models` and `/models`
pub(crate) async fn openai_models(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let data = st.service.list_models().await.map_err(ApiError)?;
    Ok(Json(ModelsResponse {
        object: "list".to_string(),
        data,
    })
    .into_response())
}

/// `GET /api/tags` (Ollama)
pub(crate) async fn ollama_tags(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let tags = st
        .service
        .list_models()
        .await
        .map_err(ApiError)?
        .into_iter()
        .map(|m: Model| OllamaModelTag {
            name: m.id.clone(),
            model: m.id,
            modified_at: rfc3339_now(),
            size: 0,
            digest: String::new(),
            details: OllamaModelDetails {
                format: "gguf".to_string(),
                ..Default::default()
            },
        })
        .collect();
    Ok(Json(OllamaTagsResponse { models: tags }).into_response())
}

/// `POST /v1/chat/completions` and `/chat/completions`
pub(crate) async fn openai_chat(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<ChatCompletionRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let run_context = RunContext::from_request(&st, &req).map_err(ApiError)?;
    let service = service_for_run(&st, &run_context);

    let model = req.model.clone();
    let want_stream = req.wants_stream();
    let include_usage = req
        .stream_options
        .as_ref()
        .and_then(|o| o.include_usage)
        .unwrap_or(false);

    let mut creq = openai_to_completion(req);
    add_workspace_instructions_for(&mut creq.messages, run_context.workspace());
    let ctx = ChunkCtx {
        id: gen_id("chatcmpl"),
        created: now_unix(),
        model: model.clone(),
    };

    if want_stream {
        let inner = service.stream(creq).await.map_err(ApiError)?;
        let stream = openai_sse(inner, ctx, include_usage);
        Ok(Sse::new(stream)
            .keep_alive(KeepAlive::default())
            .into_response())
    } else {
        let out = service.complete(creq).await.map_err(ApiError)?;
        let resp = ChatCompletionResponse {
            id: ctx.id,
            object: "chat.completion".to_string(),
            created: ctx.created,
            model,
            choices: vec![Choice {
                index: 0,
                message: out.message,
                finish_reason: Some(out.finish_reason),
            }],
            usage: out.usage,
            system_fingerprint: None,
        };
        Ok(Json(resp).into_response())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum PromptInput {
    Text(String),
    Many(Vec<String>),
}

impl PromptInput {
    fn text(self) -> String {
        match self {
            Self::Text(text) => text,
            Self::Many(items) => items.join("\n"),
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct LegacyCompletionRequest {
    model: String,
    prompt: PromptInput,
    #[serde(default)]
    suffix: Option<String>,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    top_p: Option<f32>,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    stream: Option<bool>,
    #[serde(default)]
    stop: Option<StringOrArray>,
    #[serde(default)]
    frequency_penalty: Option<f32>,
    #[serde(default)]
    presence_penalty: Option<f32>,
    #[serde(default)]
    seed: Option<i64>,
    #[serde(default)]
    echo: Option<bool>,
}

impl LegacyCompletionRequest {
    fn wants_stream(&self) -> bool {
        self.stream.unwrap_or(false)
    }
}

/// `POST /v1/completions` and `/completions`
pub(crate) async fn openai_completions(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<LegacyCompletionRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;

    let model = req.model.clone();
    let want_stream = req.wants_stream();
    let prompt = req.prompt.clone().text();
    let echo = req.echo.unwrap_or(false);
    let creq = legacy_completion_to_completion(req);
    let id = gen_id("cmpl");
    let created = now_unix();

    if want_stream {
        let inner = st.service.stream(creq).await.map_err(ApiError)?;
        return Ok(Sse::new(completion_sse(inner, id, created, model))
            .keep_alive(KeepAlive::default())
            .into_response());
    }

    let out = st.service.complete(creq).await.map_err(ApiError)?;
    let text = if echo {
        format!("{prompt}{}", out.message.text_content())
    } else {
        out.message.text_content()
    };
    Ok(Json(json!({
        "id": id,
        "object": "text_completion",
        "created": created,
        "model": model,
        "choices": [{
            "text": text,
            "index": 0,
            "logprobs": null,
            "finish_reason": out.finish_reason,
        }],
        "usage": out.usage,
    }))
    .into_response())
}

fn legacy_completion_to_completion(req: LegacyCompletionRequest) -> CompletionRequest {
    let prompt = req.prompt.text();
    CompletionRequest {
        model: req.model,
        messages: vec![ChatMessage::text("user", prompt.clone())],
        tools: Vec::new(),
        tool_choice: None,
        response_format: None,
        prompt: Some(prompt),
        suffix: req.suffix,
        sampling: SamplingParams {
            temperature: req.temperature,
            top_p: req.top_p,
            max_tokens: req.max_tokens,
            stop: req.stop.map(|s| s.into_vec()).unwrap_or_default(),
            seed: req.seed,
            frequency_penalty: req.frequency_penalty,
            presence_penalty: req.presence_penalty,
        },
        reasoning_effort: None,
    }
}

fn completion_sse(
    mut inner: EventStream,
    id: String,
    created: u64,
    model: String,
) -> impl futures::Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        while let Some(ev) = inner.next().await {
            match ev {
                Ok(StreamEvent::Delta(delta)) => {
                    if let Some(text) = delta.content {
                        yield Ok(Event::default().data(json!({
                            "id": id,
                            "object": "text_completion",
                            "created": created,
                            "model": model,
                            "choices": [{
                                "text": text,
                                "index": 0,
                                "logprobs": null,
                                "finish_reason": null,
                            }],
                        }).to_string()));
                    }
                }
                Ok(StreamEvent::Done { finish_reason, usage }) => {
                    yield Ok(Event::default().data(json!({
                        "id": id,
                        "object": "text_completion",
                        "created": created,
                        "model": model,
                        "choices": [{
                            "text": "",
                            "index": 0,
                            "logprobs": null,
                            "finish_reason": finish_reason,
                        }],
                        "usage": usage,
                    }).to_string()));
                    yield Ok(Event::default().data("[DONE]"));
                    return;
                }
                Err(e) => {
                    yield Ok(Event::default().event("error").data(json!({
                        "error": { "message": e.to_string(), "type": e.code() }
                    }).to_string()));
                    yield Ok(Event::default().data("[DONE]"));
                    return;
                }
            }
        }
        yield Ok(Event::default().data("[DONE]"));
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ResponsesInput {
    Text(String),
    Items(Vec<Value>),
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponsesRequest {
    model: String,
    input: ResponsesInput,
    #[serde(default)]
    instructions: Option<String>,
    #[serde(default)]
    stream: Option<bool>,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    top_p: Option<f32>,
    #[serde(default)]
    max_output_tokens: Option<u32>,
    #[serde(default)]
    tools: Vec<Value>,
    #[serde(default)]
    tool_choice: Option<Value>,
    #[serde(default)]
    reasoning: Option<Value>,
    #[serde(default)]
    text: Option<Value>,
    #[serde(default)]
    previous_response_id: Option<String>,
}

impl ResponsesRequest {
    fn wants_stream(&self) -> bool {
        self.stream.unwrap_or(false)
    }
}

/// `POST /v1/responses`
pub(crate) async fn openai_responses(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<ResponsesRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;

    let model = req.model.clone();
    let want_stream = req.wants_stream();
    let response_id = gen_id("resp");
    let created = now_unix();
    let text = response_text_value(req.text.clone());
    let tools = req.tools.clone();
    let tool_choice = req.tool_choice.clone().unwrap_or_else(|| json!("auto"));
    let reasoning = req
        .reasoning
        .clone()
        .unwrap_or_else(|| json!({ "effort": null, "summary": null }));
    let previous_response_id = req.previous_response_id.clone();
    let response_shape = ResponseShape {
        id: response_id,
        created,
        model,
        text,
        tools,
        tool_choice,
        reasoning,
        previous_response_id,
    };
    let creq = responses_to_completion(req).map_err(ApiError)?;

    if want_stream {
        let inner = st.service.stream(creq).await.map_err(ApiError)?;
        return Ok(Sse::new(responses_sse(inner, response_shape))
            .keep_alive(KeepAlive::default())
            .into_response());
    }

    let out = st.service.complete(creq).await.map_err(ApiError)?;
    Ok(Json(response_json(
        &response_shape,
        response_output_items(&out.message),
        Some(out.usage),
    ))
    .into_response())
}

fn responses_to_completion(req: ResponsesRequest) -> Result<CompletionRequest, Error> {
    let text = response_text_value(req.text);
    let mut messages = response_input_messages(req.input)?;
    if let Some(instructions) = req.instructions.filter(|s| !s.is_empty()) {
        messages.insert(0, ChatMessage::text("system", instructions));
    }
    Ok(CompletionRequest {
        model: req.model,
        messages,
        tools: response_tools(req.tools)?,
        tool_choice: req.tool_choice,
        response_format: response_format_from_responses_text(&text),
        prompt: None,
        suffix: None,
        sampling: SamplingParams {
            temperature: req.temperature,
            top_p: req.top_p,
            max_tokens: req.max_output_tokens,
            ..Default::default()
        },
        reasoning_effort: response_reasoning_effort(req.reasoning.as_ref()),
    })
}

fn response_input_messages(input: ResponsesInput) -> Result<Vec<ChatMessage>, Error> {
    match input {
        ResponsesInput::Text(text) => Ok(vec![ChatMessage::text("user", text)]),
        ResponsesInput::Items(items) => items.into_iter().map(response_input_item).collect(),
    }
}

fn response_input_item(item: Value) -> Result<ChatMessage, Error> {
    if let Some(text) = item.as_str() {
        return Ok(ChatMessage::text("user", text));
    }
    let kind = item
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("message");
    match kind {
        "function_call_output" => Ok(ChatMessage {
            role: "tool".to_string(),
            content: Some(Content::Text(
                item.get("output")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            )),
            name: None,
            tool_calls: None,
            tool_call_id: item
                .get("call_id")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            reasoning_content: None,
        }),
        "function_call" => Ok(ChatMessage {
            role: "assistant".to_string(),
            content: None,
            name: None,
            tool_calls: Some(vec![ToolCall {
                id: item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                kind: "function".to_string(),
                function: FunctionCall {
                    name: item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    arguments: item
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                },
            }]),
            tool_call_id: None,
            reasoning_content: None,
        }),
        _ => {
            let role = item
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("user")
                .to_string();
            let content = item
                .get("content")
                .map(response_content)
                .transpose()?
                .unwrap_or_else(|| Content::Text(String::new()));
            Ok(ChatMessage {
                role,
                content: Some(content),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
            })
        }
    }
}

fn response_content(value: &Value) -> Result<Content, Error> {
    if let Some(text) = value.as_str() {
        return Ok(Content::Text(text.to_string()));
    }
    let parts = value.as_array().ok_or_else(|| {
        Error::InvalidRequest("Responses message content must be a string or array".to_string())
    })?;
    let mut out = Vec::new();
    for part in parts {
        match part.get("type").and_then(Value::as_str) {
            Some("input_text") | Some("text") | Some("output_text") => {
                out.push(ContentPart::Text {
                    text: part
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                });
            }
            Some("input_image") | Some("image_url") => {
                if let Some(url) = part
                    .get("image_url")
                    .and_then(Value::as_str)
                    .or_else(|| part.pointer("/image_url/url").and_then(Value::as_str))
                {
                    out.push(ContentPart::ImageUrl {
                        image_url: ImageUrl {
                            url: url.to_string(),
                            detail: part
                                .get("detail")
                                .and_then(Value::as_str)
                                .map(ToString::to_string),
                        },
                    });
                }
            }
            _ => {}
        }
    }
    Ok(Content::Parts(out))
}

fn response_tools(tools: Vec<Value>) -> Result<Vec<OpenAiTool>, Error> {
    let mut out = Vec::new();
    for tool in tools {
        if tool.get("type").and_then(Value::as_str) != Some("function") {
            continue;
        }
        if tool.get("function").is_some() {
            out.push(serde_json::from_value(tool).map_err(|e| {
                Error::InvalidRequest(format!("invalid Responses function tool: {e}"))
            })?);
            continue;
        }
        let name = tool.get("name").and_then(Value::as_str).ok_or_else(|| {
            Error::InvalidRequest("Responses function tool is missing name".to_string())
        })?;
        out.push(OpenAiTool {
            kind: "function".to_string(),
            function: ToolFunction {
                name: name.to_string(),
                description: tool
                    .get("description")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                parameters: tool.get("parameters").cloned(),
            },
        });
    }
    Ok(out)
}

fn response_reasoning_effort(reasoning: Option<&Value>) -> Option<ReasoningEffort> {
    let effort = reasoning?.get("effort")?.as_str()?;
    serde_json::from_value(Value::String(effort.to_string())).ok()
}

fn response_text_value(text: Option<Value>) -> Value {
    text.unwrap_or_else(|| json!({ "format": { "type": "text" } }))
}

fn response_format_from_responses_text(text: &Value) -> Option<Value> {
    let format = text.get("format")?;
    match format.get("type").and_then(Value::as_str) {
        Some("text") | None => None,
        Some("json_schema") if format.get("json_schema").is_none() => Some(json!({
            "type": "json_schema",
            "json_schema": {
                "name": format
                    .get("name")
                    .cloned()
                    .unwrap_or_else(|| Value::String("response".to_string())),
                "strict": format.get("strict").cloned().unwrap_or(Value::Bool(false)),
                "schema": format.get("schema").cloned().unwrap_or_else(|| json!({})),
            }
        })),
        _ => Some(format.clone()),
    }
}

fn response_output_items(message: &ChatMessage) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(calls) = &message.tool_calls {
        for call in calls {
            out.push(json!({
                "type": "function_call",
                "id": call.id.clone().unwrap_or_else(|| gen_id("fc")),
                "call_id": call.id.clone().unwrap_or_else(|| gen_id("call")),
                "name": call.function.name.clone(),
                "arguments": call.function.arguments.clone(),
                "status": "completed",
            }));
        }
    }
    let text = message.text_content();
    if !text.is_empty() || out.is_empty() {
        out.push(response_message_item(&text));
    }
    out
}

fn response_message_item(text: &str) -> Value {
    json!({
        "type": "message",
        "id": gen_id("msg"),
        "status": "completed",
        "role": "assistant",
        "content": [{
            "type": "output_text",
            "text": text,
            "annotations": [],
        }],
    })
}

#[derive(Debug, Clone)]
struct ResponseShape {
    id: String,
    created: u64,
    model: String,
    text: Value,
    tools: Vec<Value>,
    tool_choice: Value,
    reasoning: Value,
    previous_response_id: Option<String>,
}

fn response_json(shape: &ResponseShape, output: Vec<Value>, usage: Option<Usage>) -> Value {
    let usage = usage.map(|u| {
        json!({
            "input_tokens": u.prompt_tokens,
            "output_tokens": u.completion_tokens,
            "total_tokens": u.total_tokens,
        })
    });
    json!({
        "id": shape.id.clone(),
        "object": "response",
        "created_at": shape.created,
        "status": "completed",
        "completed_at": now_unix(),
        "error": null,
        "incomplete_details": null,
        "instructions": null,
        "max_output_tokens": null,
        "model": shape.model.clone(),
        "output": output,
        "parallel_tool_calls": true,
        "previous_response_id": shape.previous_response_id.clone(),
        "reasoning": shape.reasoning.clone(),
        "store": false,
        "temperature": null,
        "text": shape.text.clone(),
        "tool_choice": shape.tool_choice.clone(),
        "tools": shape.tools.clone(),
        "top_p": null,
        "truncation": "disabled",
        "usage": usage,
        "user": null,
        "metadata": {},
    })
}

fn responses_sse(
    mut inner: EventStream,
    shape: ResponseShape,
) -> impl futures::Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        yield named_response_event("response.created", json!({
            "type": "response.created",
            "response": response_json(
                &shape,
                Vec::new(),
                None,
            )
        }));

        let mut content = String::new();
        let mut reasoning_text = String::new();
        let mut text_started = false;
        let mut tool_calls = ToolCallAccumulator::default();

        while let Some(ev) = inner.next().await {
            match ev {
                Ok(StreamEvent::Delta(delta)) => {
                    for call in delta.tool_calls {
                        tool_calls.push(call);
                    }
                    if let Some(reasoning_delta) = delta.reasoning {
                        reasoning_text.push_str(&reasoning_delta);
                        yield named_response_event("response.reasoning_text.delta", json!({
                            "type": "response.reasoning_text.delta",
                            "delta": reasoning_delta,
                        }));
                    }
                    if let Some(chunk) = delta.content {
                        if !text_started {
                            text_started = true;
                            yield named_response_event("response.output_item.added", json!({
                                "type": "response.output_item.added",
                                "output_index": 0,
                                "item": { "type": "message", "id": gen_id("msg"), "status": "in_progress", "role": "assistant", "content": [] }
                            }));
                            yield named_response_event("response.content_part.added", json!({
                                "type": "response.content_part.added",
                                "output_index": 0,
                                "content_index": 0,
                                "part": { "type": "output_text", "text": "", "annotations": [] }
                            }));
                        }
                        content.push_str(&chunk);
                        yield named_response_event("response.output_text.delta", json!({
                            "type": "response.output_text.delta",
                            "output_index": 0,
                            "content_index": 0,
                            "delta": chunk,
                        }));
                    }
                }
                Ok(StreamEvent::Done { usage, .. }) => {
                    let mut message = ChatMessage::text("assistant", content.clone());
                    let calls = tool_calls.finish();
                    if !calls.is_empty() {
                        message.tool_calls = Some(calls);
                    }
                    if !reasoning_text.is_empty() {
                        message.reasoning_content = Some(reasoning_text);
                    }
                    yield named_response_event("response.output_text.done", json!({
                        "type": "response.output_text.done",
                        "output_index": 0,
                        "content_index": 0,
                        "text": content,
                    }));
                    let output = response_output_items(&message);
                    yield named_response_event("response.completed", json!({
                        "type": "response.completed",
                        "response": response_json(
                            &shape,
                            output,
                            Some(usage),
                        )
                    }));
                    return;
                }
                Err(e) => {
                    yield named_response_event("response.failed", json!({
                        "type": "response.failed",
                        "response": {
                            "id": shape.id.clone(),
                            "object": "response",
                            "created_at": shape.created,
                            "status": "failed",
                            "error": { "message": e.to_string(), "type": e.code() },
                            "model": shape.model.clone(),
                        }
                    }));
                    return;
                }
            }
        }
    }
}

fn named_response_event(name: &str, value: Value) -> Result<Event, Infallible> {
    Ok(Event::default().event(name).data(value.to_string()))
}

/// `POST /api/chat` (Ollama)
pub(crate) async fn ollama_chat(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<OllamaChatRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;

    let model = req.model.clone();
    let want_stream = req.wants_stream();
    let mut creq = ollama_to_completion(req);
    add_workspace_instructions(&mut creq.messages, &st);

    if want_stream {
        let inner = st.service.stream(creq).await.map_err(ApiError)?;
        let body = Body::from_stream(ollama_ndjson(inner, model));
        Ok(Response::builder()
            .header(CONTENT_TYPE, "application/x-ndjson")
            .body(body)
            .expect("valid ndjson response"))
    } else {
        let out = st.service.complete(creq).await.map_err(ApiError)?;
        let resp = OllamaChatResponse {
            model,
            created_at: rfc3339_now(),
            message: OllamaMessage {
                role: "assistant".to_string(),
                content: out.message.text_content(),
                images: None,
                tool_calls: out.message.tool_calls,
                thinking: out.message.reasoning_content,
            },
            done: true,
            done_reason: Some(out.finish_reason),
            total_duration: Some(0),
            prompt_eval_count: Some(out.usage.prompt_tokens),
            eval_count: Some(out.usage.completion_tokens),
        };
        Ok(Json(resp).into_response())
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct OllamaGenerateRequest {
    model: String,
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    suffix: Option<String>,
    #[serde(default)]
    system: Option<String>,
    #[serde(default)]
    stream: Option<bool>,
    #[serde(default)]
    raw: Option<bool>,
    #[serde(default)]
    format: Option<Value>,
    #[serde(default)]
    keep_alive: Option<Value>,
    #[serde(default)]
    options: Option<Value>,
    #[serde(default)]
    think: Option<Value>,
}

impl OllamaGenerateRequest {
    fn wants_stream(&self) -> bool {
        self.stream.unwrap_or(true)
    }
}

/// `POST /api/generate` (Ollama)
pub(crate) async fn ollama_generate(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<OllamaGenerateRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;

    let model = req.model.clone();
    let want_stream = req.wants_stream();
    if req.prompt.is_empty() && req.keep_alive.is_some() {
        let keep_alive = req.keep_alive.clone();
        st.service
            .ollama_keep_alive(&model, keep_alive.as_ref().cloned())
            .await
            .map_err(ApiError)?;
        let done_reason = ollama_keep_alive_done_reason(keep_alive.as_ref());
        let value = ollama_keep_alive_response(&model, done_reason);
        if want_stream {
            let body = Body::from(ollama_generate_line(value));
            return Ok(Response::builder()
                .header(CONTENT_TYPE, "application/x-ndjson")
                .body(body)
                .expect("valid ndjson response"));
        }
        return Ok(Json(value).into_response());
    }
    let creq = ollama_generate_to_completion(req);

    if want_stream {
        let inner = st.service.stream(creq).await.map_err(ApiError)?;
        let body = Body::from_stream(ollama_generate_ndjson(inner, model));
        return Ok(Response::builder()
            .header(CONTENT_TYPE, "application/x-ndjson")
            .body(body)
            .expect("valid ndjson response"));
    }

    let out = st.service.complete(creq).await.map_err(ApiError)?;
    Ok(Json(json!({
        "model": model,
        "created_at": rfc3339_now(),
        "response": out.message.text_content(),
        "thinking": out.message.reasoning_content,
        "done": true,
        "done_reason": out.finish_reason,
        "total_duration": 0,
        "load_duration": 0,
        "prompt_eval_count": out.usage.prompt_tokens,
        "prompt_eval_duration": 0,
        "eval_count": out.usage.completion_tokens,
        "eval_duration": 0,
    }))
    .into_response())
}

fn ollama_generate_to_completion(req: OllamaGenerateRequest) -> CompletionRequest {
    let opts = req.options.unwrap_or(Value::Null);
    let mut messages = Vec::new();
    if !req.raw.unwrap_or(false) {
        if let Some(system) = req.system.filter(|s| !s.is_empty()) {
            messages.push(ChatMessage::text("system", system));
        }
    }
    messages.push(ChatMessage::text("user", req.prompt.clone()));
    CompletionRequest {
        model: req.model,
        messages,
        tools: Vec::new(),
        tool_choice: None,
        response_format: req.format.map(ollama_format_to_response_format),
        prompt: Some(req.prompt),
        suffix: req.suffix,
        sampling: SamplingParams {
            temperature: opt_f32(&opts, "temperature"),
            top_p: opt_f32(&opts, "top_p"),
            max_tokens: opt_u32(&opts, "num_predict"),
            stop: opt_stops(&opts),
            seed: opt_i64(&opts, "seed"),
            frequency_penalty: opt_f32(&opts, "frequency_penalty"),
            presence_penalty: opt_f32(&opts, "presence_penalty"),
        },
        reasoning_effort: ollama_think_effort(req.think.as_ref()),
    }
}

fn ollama_keep_alive_done_reason(keep_alive: Option<&Value>) -> &'static str {
    match keep_alive {
        Some(Value::Number(n)) if n.as_i64() == Some(0) || n.as_u64() == Some(0) => "unload",
        Some(Value::String(s)) if s.trim() == "0" => "unload",
        _ => "load",
    }
}

fn ollama_keep_alive_response(model: &str, done_reason: &str) -> Value {
    json!({
        "model": model,
        "created_at": rfc3339_now(),
        "response": "",
        "done": true,
        "done_reason": done_reason,
        "total_duration": 0,
        "load_duration": 0,
        "prompt_eval_count": 0,
        "prompt_eval_duration": 0,
        "eval_count": 0,
        "eval_duration": 0,
    })
}

fn ollama_generate_ndjson(
    mut inner: EventStream,
    model: String,
) -> impl futures::Stream<Item = Result<Bytes, std::io::Error>> {
    async_stream::stream! {
        while let Some(ev) = inner.next().await {
            match ev {
                Ok(StreamEvent::Delta(delta)) => {
                    if let Some(content) = delta.content {
                        yield Ok(ollama_generate_line(json!({
                            "model": model,
                            "created_at": rfc3339_now(),
                            "response": content,
                            "done": false,
                        })));
                    }
                    if let Some(thinking) = delta.reasoning {
                        yield Ok(ollama_generate_line(json!({
                            "model": model,
                            "created_at": rfc3339_now(),
                            "response": "",
                            "thinking": thinking,
                            "done": false,
                        })));
                    }
                }
                Ok(StreamEvent::Done { finish_reason, usage }) => {
                    yield Ok(ollama_generate_line(json!({
                        "model": model,
                        "created_at": rfc3339_now(),
                        "response": "",
                        "done": true,
                        "done_reason": finish_reason,
                        "total_duration": 0,
                        "load_duration": 0,
                        "prompt_eval_count": usage.prompt_tokens,
                        "prompt_eval_duration": 0,
                        "eval_count": usage.completion_tokens,
                        "eval_duration": 0,
                    })));
                    return;
                }
                Err(e) => {
                    yield Ok(ollama_generate_line(json!({
                        "model": model,
                        "created_at": rfc3339_now(),
                        "response": "",
                        "done": true,
                        "done_reason": format!("error: {e}"),
                    })));
                    return;
                }
            }
        }
    }
}

fn ollama_generate_line(value: Value) -> Bytes {
    let mut line = value.to_string();
    line.push('\n');
    Bytes::from(line)
}

fn opt_f32(v: &Value, key: &str) -> Option<f32> {
    v.get(key).and_then(|x| x.as_f64()).map(|x| x as f32)
}

fn opt_u32(v: &Value, key: &str) -> Option<u32> {
    v.get(key).and_then(|x| x.as_u64()).map(|x| x as u32)
}

fn opt_i64(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(|x| x.as_i64())
}

fn opt_stops(v: &Value) -> Vec<String> {
    match v.get("stop") {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

/// `POST /anthropic/v1/messages`
pub(crate) async fn anthropic_messages(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<MessagesRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;

    let model = req.model.clone();
    let want_stream = req.wants_stream();
    let creq = anthropic_to_completion(req).map_err(ApiError)?;
    let id = gen_id("msg");

    if want_stream {
        let inner = st.service.stream(creq).await.map_err(ApiError)?;
        let stream = anthropic_sse(inner, id, model);
        Ok(Sse::new(stream)
            .keep_alive(KeepAlive::default())
            .into_response())
    } else {
        let out = st.service.complete(creq).await.map_err(ApiError)?;
        let resp = MessagesResponse {
            id,
            kind: "message".to_string(),
            role: "assistant".to_string(),
            content: anthropic_response_blocks(&out.message),
            model,
            stop_reason: Some(anthropic_stop_reason(&out.finish_reason)),
            stop_sequence: None,
            usage: anthropic::Usage {
                input_tokens: out.usage.prompt_tokens,
                output_tokens: out.usage.completion_tokens,
            },
        };
        Ok(Json(resp).into_response())
    }
}
