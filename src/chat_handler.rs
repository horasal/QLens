use std::sync::Arc;

use crate::{
    ChatEntry, FN_MAX_LEN, FN_STOP_WORDS,
    schema::{Message, MessageContent, Role, ToolUse},
    tools::{FN_ARGS, FN_EXIT, FN_NAME, FN_RESULT, ToolSet},
};
use anyhow::{Error, anyhow};
use async_openai::types::{
    ChatCompletionRequestAssistantMessage, ChatCompletionRequestAssistantMessageContent,
    ChatCompletionRequestAssistantMessageContentPart, ChatCompletionRequestMessageContentPartImage,
    ChatCompletionRequestMessageContentPartText, ChatCompletionRequestSystemMessage,
    ChatCompletionRequestSystemMessageContent, ChatCompletionRequestToolMessage,
    ChatCompletionRequestToolMessageContent, ChatCompletionRequestToolMessageContentPart,
    ChatCompletionRequestUserMessage, ChatCompletionRequestUserMessageContent,
};

use async_openai::{
    config::Config,
    types::{
        ChatCompletionRequestMessage, ChatCompletionRequestUserMessageContentPart,
        CreateChatCompletionRequest, ImageUrl,
    },
    *,
};
use async_stream::try_stream;
use base64::{Engine, prelude::BASE64_STANDARD};
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use sled::IVec;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct LLMConfig {
    pub model: Option<String>,
    pub temp: Option<f32>,
    pub stream: Option<bool>,
    pub frequency_penalty: Option<f32>,
    pub presence_penality: Option<f32>,
    pub top_p: Option<f32>,
    pub user: Option<String>,
    pub seed: Option<i64>,
    pub max_completion_tokens: Option<u32>,
    pub parallel_function_call: bool,
    pub system_prompt_lang: Option<whatlang::Lang>,
}

impl Default for LLMConfig {
    fn default() -> Self {
        Self {
            model: None,
            temp: Some(0.8),
            stream: Some(true),
            frequency_penalty: Some(1.0),
            presence_penality: None,
            top_p: None,
            user: None,
            seed: None,
            max_completion_tokens: None,
            parallel_function_call: false,
            system_prompt_lang: Some(whatlang::Lang::Cmn),
        }
    }
}

impl Into<CreateChatCompletionRequest> for LLMConfig {
    fn into(self) -> CreateChatCompletionRequest {
        let mut r = CreateChatCompletionRequest::default();
        r.model = self.model.unwrap_or("".to_string());
        r.temperature = self.temp;
        r.stream = self.stream;
        r.frequency_penalty = self.frequency_penalty;
        r.presence_penalty = self.presence_penality;
        r.top_p = self.top_p;
        r.user = self.user;
        r.seed = self.seed;
        r.max_completion_tokens = self.max_completion_tokens;
        r.n = Some(1);
        r.reasoning_effort = Some(types::ReasoningEffort::Medium);
        r
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChatEvent {
    /// 对应 ✿FUNCTION✿ 之前的所有文本。
    ReasoningDelta(String),

    /// 一个正在输出的工具调用
    /// 本身除了跟Reasoning和Content区别开之外没有别的意义
    /// 最终的工具调用需要使用ToolCall像前端展示
    ToolDelta(String),

    /// 一个完整的工具调用已被解析。
    /// 前端可以用这个显示“正在调用 [image_zoom_in_tool]...”。
    ToolCall(ToolUse),

    /// 一个工具已执行完毕，这是它的（可显示的）结果。
    ToolResult {
        /// 对应的工具调用（用于UI关联）
        tool_use: ToolUse,
        /// 结果 (LLMProvider 应从DB中提取Blob并填充)
        result: Vec<MessageContent>,
    },

    /// LLM的“最终回复”的增量。
    ContentDelta(String),
    /// 通知UI已经结束
    StreamEnd {},
}

#[derive(Debug, Clone, Copy)]
enum StreamParseState {
    /// 初始状态, 正在“窥视” FN_NAME 或等待足够的数据来决定
    AwaitingDecision,
    /// 已确定这是一个纯内容（无工具）流
    /// 现在这个没有任何用处，因为一个流要不然是tool use，要不然是未来可能有tool use
    #[allow(dead_code)]
    Content,
    /// 已找到 FN_NAME, 正在解析工具名称
    ToolCallName,
    /// 已找到 FN_ARGS, 正在解析工具参数
    ToolCallArgs,
}

pub struct LLMProvider<T>
where
    T: Config,
{
    client: Arc<Client<T>>,
    pub history: sled::Tree,
    pub image: sled::Tree,
    toolset: Arc<ToolSet>,
}

impl<T: Config> Clone for LLMProvider<T> {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            history: self.history.clone(),
            image: self.image.clone(),
            toolset: self.toolset.clone(),
        }
    }
}

impl<T: Config> LLMProvider<T> {
    pub fn new(
        client: Client<T>,
        history: sled::Tree,
        image: sled::Tree,
        toolset: ToolSet,
    ) -> Result<Self, Error> {
        tracing::info!("Active tools: {}", toolset);
        Ok(Self {
            client: Arc::new(client),
            history: history,
            image: image,
            toolset: Arc::new(toolset),
        })
    }

    pub fn save_image(&self, binary: &[u8]) -> Result<Uuid, Error> {
        for _ in 0..20 {
            let uuid = Uuid::new_v4();
            if self
                .image
                .compare_and_swap(uuid, None::<&[u8]>, Some(binary))?
                .is_ok()
            {
                return Ok(uuid);
            }
        }
        Err(anyhow!("Unable to generate unique id in 20 tries."))
    }

    pub fn new_chat(&self) -> Result<ChatEntry, Error> {
        for _ in 0..20 {
            let e = ChatEntry::default();
            if self
                .history
                .compare_and_swap(e.id, None::<&[u8]>, Some(serde_json::to_vec(&e)?))?
                .is_ok()
            {
                return Ok(e);
            }
        }
        Err(anyhow!("Unable to generate unique id in 20 tries."))
    }

    pub async fn send_chat_message(
        &self,
        chat_id: Uuid,
        user_content: Vec<MessageContent>,
        llm_config: LLMConfig,
    ) -> Result<impl Stream<Item = Result<ChatEvent, Error>>, Error> {
        let user_message = Message {
            owner: Role::User,
            content: user_content,
            reasoning: vec![],
            tool_use: vec![],
        };
        let provider = self.clone();
        Ok(try_stream! {
            let mut current_session = provider.append_message(chat_id, user_message)?;
            loop {
                let req_messages = provider.message_to_openai(current_session.clone(), llm_config.parallel_function_call, llm_config.system_prompt_lang);
                let mut req: CreateChatCompletionRequest = llm_config.clone().into();
                req.messages = req_messages;

                let mut stream = self.client.chat().create_stream(req).await?;
                let mut state = StreamParseState::AwaitingDecision;
                            let mut parse_buffer: String = String::new(); // 切换回 String
                            let mut assistant_thinking = String::new();
                            let mut assistant_reasoning = String::new();
                            let mut assistant_content = String::new();
                            let mut assistant_tool_calls = Vec::new();
                            let mut current_tool_name = String::new();

                            while let Some(thunk) = stream.next().await {
                                let thunk = thunk?;
                                if let Some(content) = thunk
                                    .choices
                                    .first()
                                    .and_then(|c| c.delta.reasoning_content.as_ref())
                                {
                                    // 原生的思考不涉及tool use，不需要任何处理
                                    assistant_thinking.push_str(&content);
                                    yield ChatEvent::ReasoningDelta(content.clone());
                                    continue;
                                } else if let Some(content) = thunk.choices.first().and_then(|c| c.delta.content.as_ref())
                                {
                                    parse_buffer.push_str(content);
                                } else {
                                    continue;
                                }

                                'parse_loop: loop {
                                    match state {
                                        StreamParseState::AwaitingDecision => {
                                            if let Some(idx) = parse_buffer.find(FN_NAME) {
                                                let reasoning_part: String = parse_buffer.drain(..idx).collect();
                                                parse_buffer.drain(..FN_NAME.len());

                                                if !reasoning_part.is_empty() {
                                                    assistant_reasoning.push_str(&reasoning_part);
                                                    yield ChatEvent::ReasoningDelta(reasoning_part);
                                                }
                                                yield ChatEvent::ToolDelta(FN_NAME.to_string());
                                                state = StreamParseState::ToolCallName;
                                            } else {
                                                if parse_buffer.len() > FN_MAX_LEN {
                                                    let yield_len_bytes =
                                                        parse_buffer.len().saturating_sub(FN_MAX_LEN/2);

                                                    // 寻找安全的 UTF-8 切割点
                                                    let mut split_idx = yield_len_bytes;
                                                    while !parse_buffer.is_char_boundary(split_idx) {
                                                        split_idx -= 1;
                                                    }

                                                    if split_idx > 0 {
                                                        let content_part: String =
                                                            parse_buffer.drain(..split_idx).collect();
                                                        if assistant_reasoning.is_empty() {
                                                            assistant_content.push_str(&content_part);
                                                            yield ChatEvent::ContentDelta(content_part);
                                                        } else {
                                                            //之前有过一次函数调用，之后任何信息都是reasoning
                                                            assistant_thinking.push_str(&content_part);
                                                            yield ChatEvent::ReasoningDelta(content_part);
                                                        }
                                                    }
                                                    //这里不需要设为Content，说不定未来还有FN_NAME
                                                    //state = StreamParseState::Content;
                                                } else {
                                                    // 缓冲区中没有标签, 但数据还不够, 无法确定.
                                                    break 'parse_loop;
                                                }
                                            }
                                        }
                                        StreamParseState::Content => {
                                            // 我们已处于内容模式. 将所有内容作为 ContentDelta 发送
                                            if !parse_buffer.is_empty() {
                                                let content_part: String = parse_buffer.drain(..).collect();
                                                assistant_content.push_str(&content_part);
                                                yield ChatEvent::ContentDelta(content_part);
                                            }
                                            break 'parse_loop;
                                        }
                                        StreamParseState::ToolCallName => {
                                            if let Some(idx) = parse_buffer.find(FN_ARGS) {
                                                let name_part_raw: String = parse_buffer.drain(..idx).collect();
                                                parse_buffer.drain(..FN_ARGS.len());

                                                current_tool_name =
                                                    name_part_raw.trim()
                                                        .trim_matches(':')
                                                        .trim_matches('：')
                                                        .trim().to_string();

                                                let delta = name_part_raw + FN_ARGS;
                                                assistant_reasoning.push_str(&delta);
                                                yield ChatEvent::ToolDelta(delta);

                                                state = StreamParseState::ToolCallArgs;
                                            } else {
                                                break 'parse_loop;
                                            }
                                        }
                                        StreamParseState::ToolCallArgs => {
                                            let next_stop = FN_STOP_WORDS
                                                .iter()
                                                .filter_map(|tag| parse_buffer.find(tag).map(|idx| (idx, *tag)))
                                                .min_by_key(|(idx, _)| *idx);

                                            if let Some((idx, tag)) = next_stop {
                                                let args_part_raw: String = parse_buffer.drain(..idx).collect();
                                                parse_buffer.drain(..tag.len());

                                                let args =
                                                    args_part_raw.trim()
                                                        .trim_matches(':')
                                                        .trim_matches('：')
                                                        .trim().to_string();

                                                assistant_reasoning.push_str(&args_part_raw);
                                                yield ChatEvent::ToolDelta(args_part_raw);

                                                let tool_use =
                                                    ToolUse { function_name: current_tool_name.clone(), args };
                                                yield ChatEvent::ToolCall(tool_use.clone());
                                                assistant_tool_calls.push(tool_use);

                                                current_tool_name.clear();
                                                if tag != FN_EXIT {
                                                    yield ChatEvent::ToolDelta(tag.to_string());
                                                }

                                                if tag == FN_NAME {
                                                    state = StreamParseState::ToolCallName;
                                                } else {
                                                    // 但tool use本身就是reasoning，因此这里不是content
                                                    state = StreamParseState::AwaitingDecision;
                                                }
                                            } else {
                                                break 'parse_loop;
                                            }
                                        }
                                    }
                                }
                            }
                match state {
                    StreamParseState::AwaitingDecision | StreamParseState::Content | StreamParseState::ToolCallName=> {
                        // 剩余的都是内容
                        if !parse_buffer.is_empty() {
                            if assistant_reasoning.is_empty() {
                                assistant_content.push_str(&parse_buffer);
                                yield ChatEvent::ContentDelta(parse_buffer);
                            } else {
                                assistant_thinking.push_str(&parse_buffer);
                                yield ChatEvent::ReasoningDelta(parse_buffer);
                            }
                        }
                    }
                    StreamParseState::ToolCallArgs => {
                        // 这是最后一个工具的参数
                        let args = parse_buffer.trim()
                            .trim_matches(':')
                            .trim_matches('：')
                            .trim().to_string();
                        assistant_reasoning.push_str(&parse_buffer);
                        yield ChatEvent::ToolDelta(parse_buffer);

                        let tool_use = ToolUse { function_name: current_tool_name.clone(), args };
                        yield ChatEvent::ToolCall(tool_use.clone());
                        assistant_tool_calls.push(tool_use);
                    }
                }
                let assistant_message = Message {
                        owner: Role::Assistant,
                        reasoning: {
                            let mut v = vec![];
                            if !assistant_thinking.is_empty() {
                                v.push(MessageContent::Text(assistant_thinking));
                            }
                            // assistant_reasoning已经被解析成了tooluse，不需要保留
                            // 否则会在gui里显示丑陋的原始输出
                            //if !assistant_reasoning.is_empty() {
                            //    v.push(MessageContent::Text(assistant_reasoning));
                            //}
                            v
                        },
                        content: if !assistant_content.is_empty() { vec![MessageContent::Text(assistant_content)] } else { vec![] },
                        tool_use: assistant_tool_calls.clone(),
                    };

                current_session = provider.append_message(chat_id, assistant_message)?;

                if assistant_tool_calls.is_empty() {
                    // 没有工具调用 (这是一个内容流)
                    break;
                }

                let mut futures = Vec::new();
                for tool_call in assistant_tool_calls.iter() {
                    futures.push(provider.toolset.use_tool_async(tool_call.function_name.clone(), tool_call.args.clone()));
                }

                let results: Vec<Message> = futures::future::join_all(futures).await;

                let mut tool_results_content = Vec::new();
                for (i, res) in results.into_iter().enumerate() {
                    let tool_use = assistant_tool_calls[i].clone();
                    let result_content = res;

                    yield ChatEvent::ToolResult { tool_use, result: result_content.content.clone() };
                    tool_results_content.push(result_content);
                }

                let tool_message = Message {
                    owner: Role::Tools,
                    content: tool_results_content.into_iter().flat_map(|v| v.content).collect(),
                    reasoning: vec![],
                    tool_use: vec![],
                };

                current_session = provider.append_message(chat_id, tool_message)?;
            }
        })
    }

    #[allow(dead_code)]
    fn hydrate_image_ref(&self, content: &MessageContent) -> Result<MessageContent, Error> {
        match content {
            MessageContent::ImageRef(id, label) => {
                let image_data = self
                    .image
                    .get(id)?
                    .ok_or(anyhow::anyhow!("Image {} not in DB", id))?;
                Ok(MessageContent::ImageBin(
                    image_data.to_vec(),
                    id.clone(),
                    label.clone(),
                ))
            }
            _ => Ok(content.clone()), // 已经是 Text 或 Bin, 直接返回
        }
    }

    fn append_message(&self, chat_id: Uuid, content: Message) -> Result<ChatEntry, Error> {
        let old_buf = self.history.get(chat_id)?;
        let mut current_buf = append_message_to_buffer(chat_id, &old_buf, &content)?;
        for _ in 0..10 {
            match self.history.compare_and_swap(
                chat_id,
                old_buf.clone(),
                Some(current_buf.clone()),
            )? {
                Ok(()) => break,
                Err(e) => {
                    tracing::warn!(
                        "Chat Session {} modified during append new message, try again.",
                        chat_id
                    );
                    current_buf = append_message_to_buffer(chat_id, &e.current, &content)?;
                }
            }
        }
        Ok(serde_json::from_slice(&current_buf)?)
    }

    fn message_to_openai(
        &self,
        v: ChatEntry,
        is_parallel_fc: bool,
        lang: Option<whatlang::Lang>
    ) -> Vec<ChatCompletionRequestMessage> {
        let lang = match lang {
            Some(l) => l,
            None => v
            .messages
            .iter()
            .filter(|v| v.owner == Role::User)
            .map(|v| {
                v.content
                    .iter()
                    .filter_map(|v| match v {
                        MessageContent::Text(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect::<Vec<String>>()
                    .join("")
            })
            .filter(|v| v.len() > 0)
            .last()
            .map(|v| whatlang::detect_lang(&v))
            .flatten()
            .unwrap_or(whatlang::Lang::Cmn)
        };
        tracing::info!("User input language: {}", lang);
        let system_message =
            ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
                content: ChatCompletionRequestSystemMessageContent::Text(
                    self.toolset.system_prompt(lang, is_parallel_fc),
                ),
                name: None,
            });
        let mut history_messages: Vec<ChatCompletionRequestMessage> = v
            .messages
            .into_iter()
            .map(|v| self.message_to_request(v))
            .filter_map(|v| v.ok())
            .collect();
        history_messages.insert(0, system_message);

        history_messages
    }

    fn message_to_request(&self, v: Message) -> Result<ChatCompletionRequestMessage, Error> {
        Ok(match v.owner {
            Role::Assistant => ChatCompletionRequestMessage::Assistant({
                let mut r = ChatCompletionRequestAssistantMessage::default();
                r.content = Some(ChatCompletionRequestAssistantMessageContent::Array(
                    v.content
                        .into_iter()
                        .map(|v| v.into())
                        .chain(v.tool_use.into_iter().map(|v| {
                            ChatCompletionRequestAssistantMessageContentPart::Text(
                                ChatCompletionRequestMessageContentPartText {
                                    text: format!(
                                        "\n{FN_NAME}: {fn_name}\n{FN_ARGS}: {fn_args}\n",
                                        fn_name = v.function_name,
                                        fn_args = v.args, // remove to prevent repeat?
                                    ),
                                },
                            )
                        }))
                        .collect(),
                ));
                r
            }),
            Role::System => ChatCompletionRequestMessage::System({
                let mut r = ChatCompletionRequestSystemMessage::default();
                r.name = Some("System".to_string());
                r.content = ChatCompletionRequestSystemMessageContent::Array(
                    v.content.into_iter().map(|v| v.into()).collect(),
                );
                r
            }),
            Role::User => ChatCompletionRequestMessage::User({
                let mut r = ChatCompletionRequestUserMessage::default();
                r.content = ChatCompletionRequestUserMessageContent::Array(
                    self.map_multi_modal_user_messages(v)?,
                );
                r
            }),
            Role::Tools => ChatCompletionRequestMessage::Tool({
                //原始的async-openai不支持tool返回image，这里用的是修改版
                //let mut r = ChatCompletionRequestUserMessage::default();
                //r.content = ChatCompletionRequestUserMessageContent::Array(
                //    self.map_multi_modal_tool_messages(v)?,
                //);
                let mut r = ChatCompletionRequestToolMessage::default();
                r.content = ChatCompletionRequestToolMessageContent::Array(
                    self.map_multi_modal_tool_messages(v)?,
                );
                r
            }),
        })
    }

    fn map_multi_modal_tool_messages(
        &self,
        v: Message,
    ) -> Result<Vec<ChatCompletionRequestToolMessageContentPart>, Error> {
        let mut res = Vec::new();

        // 添加 ✿RESULT✿: 前缀
        res.push(ChatCompletionRequestToolMessageContentPart::Text(
            ChatCompletionRequestMessageContentPartText {
                text: format!("{FN_RESULT}: "),
            },
        ));

        for msg in v.content {
            match msg {
                MessageContent::Text(text) => {
                    // 如果工具只返回文本 (例如错误信息)
                    res.push(ChatCompletionRequestToolMessageContentPart::Text(
                        ChatCompletionRequestMessageContentPartText { text },
                    ));
                }
                MessageContent::ImageRef(id, _) => {
                    // 添加文本描述 (LLM可以读取这个UUID和标签)
                    res.push(ChatCompletionRequestToolMessageContentPart::Text(
                        ChatCompletionRequestMessageContentPartText {
                            text: msg.to_string(),
                        },
                    ));

                    match self.image.get(id).map(|v| {
                        v.map(|v| format!("data:image/png;base64,{}", BASE64_STANDARD.encode(&v)))
                    }) {
                        Ok(Some(b)) => {
                            res.push(ChatCompletionRequestToolMessageContentPart::ImageUrl(
                                ChatCompletionRequestMessageContentPartImage {
                                    image_url: ImageUrl {
                                        url: b,
                                        detail: None,
                                    },
                                },
                            ));
                        }
                        Err(e) => {
                            tracing::warn!("Unable to get tool result image {} from database: {}, skip.", id, e)
                        }
                        Ok(None) => tracing::warn!("Tool result image {} not in database, skip.", id),
                    }
                }
                MessageContent::ImageBin(ref blob, _, _) => {
                    res.push(ChatCompletionRequestToolMessageContentPart::Text(
                        ChatCompletionRequestMessageContentPartText {
                            text: msg.to_string(),
                        },
                    ));

                    let b = format!("data:image/png;base64,{}", BASE64_STANDARD.encode(blob));
                    res.push(ChatCompletionRequestToolMessageContentPart::ImageUrl(
                        ChatCompletionRequestMessageContentPartImage {
                            image_url: ImageUrl {
                                url: b,
                                detail: None,
                            },
                        },
                    ));
                }
            }
        }

        // 添加 ✿RETURN✿: 后缀
        res.push(ChatCompletionRequestToolMessageContentPart::Text(
            ChatCompletionRequestMessageContentPartText {
                text: format!("\n{FN_EXIT}\n"),
            },
        ));

        Ok(res)
    }

    fn map_multi_modal_user_messages(
        &self,
        v: Message,
    ) -> Result<Vec<ChatCompletionRequestUserMessageContentPart>, Error> {
        let mut res = Vec::new();
        for msg in v.content {
            match msg {
                MessageContent::Text(_) => {
                    res.push(ChatCompletionRequestUserMessageContentPart::Text(
                        types::ChatCompletionRequestMessageContentPartText {
                            text: msg.to_string(),
                        },
                    ))
                }
                MessageContent::ImageRef(id, _) => {
                    match self.image.get(id).map(|v| {
                        v.map(|v| format!("data:image/png;base64,{}", BASE64_STANDARD.encode(&v)))
                    }) {
                        Ok(Some(b)) => {
                            res.push(ChatCompletionRequestUserMessageContentPart::Text(
                                types::ChatCompletionRequestMessageContentPartText {
                                    text: msg.to_string(),
                                },
                            ));
                            res.push(ChatCompletionRequestUserMessageContentPart::ImageUrl(
                                types::ChatCompletionRequestMessageContentPartImage {
                                    image_url: ImageUrl {
                                        url: b,
                                        detail: None,
                                    },
                                },
                            ));
                        }
                        Err(e) => {
                            tracing::warn!("unable to get image {} from database {}, skip.", id, e)
                        }
                        Ok(None) => {
                            tracing::warn!("image {} does not exist in database, skip.", id)
                        }
                    }
                }
                MessageContent::ImageBin(ref blob, _, _) => {
                    let b = format!("data:image/png;base64,{}", BASE64_STANDARD.encode(&blob));
                    res.push(ChatCompletionRequestUserMessageContentPart::Text(
                        types::ChatCompletionRequestMessageContentPartText {
                            text: msg.to_string(),
                        },
                    ));
                    res.push(ChatCompletionRequestUserMessageContentPart::ImageUrl(
                        types::ChatCompletionRequestMessageContentPartImage {
                            image_url: ImageUrl {
                                url: b,
                                detail: None,
                            },
                        },
                    ));
                }
            };
        }
        Ok(res)
    }
}

fn append_message_to_buffer(
    chat_id: Uuid,
    old_buf: &Option<IVec>,
    content: &Message,
) -> Result<Vec<u8>, Error> {
    let mut vec: ChatEntry = match old_buf {
        Some(buf) => {
            if buf.len() > 1 {
                serde_json::from_slice(buf)?
            } else {
                let mut c = ChatEntry::default();
                c.id = chat_id;
                c
            }
        }
        None => {
            tracing::info!("Chat Session {} does not exist, creating.", chat_id);
            let mut c = ChatEntry::default();
            c.id = chat_id;
            c
        }
    };
    if content.owner == Role::User && vec.summary.len() == 0 {
        vec.summary = content
            .content
            .iter()
            .map(|v| match v {
                MessageContent::Text(s) => s.clone(),
                MessageContent::ImageBin(_, _, _) | MessageContent::ImageRef(_, _) => {
                    format!("[IMG]",)
                }
            })
            .collect::<String>()
            .chars()
            .take(64)
            .collect::<String>();
    }
    vec.messages.push(content.clone());
    serde_json::to_vec(&vec).map_err(|e| e.into())
}
