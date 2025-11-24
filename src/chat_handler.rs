use std::{collections::HashSet, sync::Arc};

use crate::{
    ChatEntry, ChatMeta, FN_MAX_LEN, FN_STOP_WORDS, ToolDescription, ToolKind,
    blob::{BlobStorage, SledBlobStorage},
    schema::{Message, MessageContent, Role, ToolUse},
    tools::{FN_ARGS, FN_EXIT, FN_NAME, FN_RESULT, ToolSet},
};
use anyhow::{Error, anyhow, bail};
use async_openai::types::{
    ChatCompletionRequestAssistantMessage, ChatCompletionRequestAssistantMessageContent,
    ChatCompletionRequestAssistantMessageContentPart, ChatCompletionRequestMessageContentPartImage,
    ChatCompletionRequestMessageContentPartText, ChatCompletionRequestSystemMessage,
    ChatCompletionRequestSystemMessageContent, ChatCompletionRequestToolMessage,
    ChatCompletionRequestToolMessageContent, ChatCompletionRequestToolMessageContentPart,
    ChatCompletionRequestUserMessage, ChatCompletionRequestUserMessageContent,
    ChatCompletionStreamOptions, CompletionUsage,
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
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    pub parallel_function_call: Option<bool>,
    pub system_prompt_lang: Option<whatlang::Lang>,
    pub custom_system_prompt: Option<String>,
}

impl LLMConfig {
    /// Merge `other` into self
    /// a member will be keep as `self` if it exists in `self`
    /// if it's none in `self` will be applied from `other`
    pub fn merge_in(self, other: Self) -> Self {
        Self {
            model: self.model.or(other.model),
            temp: self.temp.or(other.temp),
            stream: self.stream.or(other.stream),
            frequency_penalty: self.frequency_penalty.or(other.frequency_penalty),
            presence_penality: self.presence_penality.or(other.presence_penality),
            top_p: self.top_p.or(other.top_p),
            user: self.user.or(other.user),
            seed: self.seed.or(other.seed),
            max_completion_tokens: self.max_completion_tokens.or(other.max_completion_tokens),
            parallel_function_call: self.parallel_function_call.or(other.parallel_function_call),
            system_prompt_lang: self.system_prompt_lang.or(other.system_prompt_lang),
            custom_system_prompt: self.custom_system_prompt.or(other.custom_system_prompt),
        }
    }
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
            parallel_function_call: None,
            system_prompt_lang: Some(whatlang::Lang::Cmn),
            custom_system_prompt: None,
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
        result: Message,
    },

    /// LLM的“最终回复”的增量。
    ContentDelta(String),
    /// 通知UI已经结束
    StreamEnd {},
    /// Token数量的通知
    Usage(CompletionUsage),
    /// 服务器错误信息
    Error(String),
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
    history: sled::Tree,
    image: Arc<dyn BlobStorage>,
    asset: Arc<dyn BlobStorage>,
    memo: Arc<dyn BlobStorage>,
    toolset: Arc<ToolSet>,
}

impl<T: Config> Clone for LLMProvider<T> {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            history: self.history.clone(),
            image: self.image.clone(),
            asset: self.asset.clone(),
            memo: self.memo.clone(),
            toolset: self.toolset.clone(),
        }
    }
}

impl<T: Config> LLMProvider<T> {
    pub fn new(client: Client<T>, db_path: &str, active_tools: &[ToolKind]) -> Result<Self, Error> {
        let db = sled::Config::new()
            .temporary(false)
            .path(db_path)
            .use_compression(true)
            .open()?;
        let history_db = db.open_tree("history")?;
        let image = Arc::new(SledBlobStorage::new_from_db(&db, "image")?);
        let asset = Arc::new(SledBlobStorage::new_from_db(&db, "asset")?);
        let memo = Arc::new(SledBlobStorage::new_from_db(&db, "memo")?);
        tracing::info!("DB started.");
        let active_tools = active_tools
            .iter()
            .cloned()
            .chain(ToolKind::post_register_list().into_iter())
            .collect::<HashSet<ToolKind>>();
        let toolset = active_tools
            .iter()
            .map(|kind| kind.create_tool(image.clone(), asset.clone(), memo.clone()))
            .fold(ToolSet::builder(), |ts, t| ts.add_tool(t))
            .build();
        tracing::info!("Active tools: {}", toolset);
        Ok(Self {
            client: Arc::new(client),
            history: history_db,
            image: image,
            asset: asset,
            memo: memo,
            toolset: Arc::new(toolset),
        })
    }

    pub async fn get_model_names(&self) -> Result<Vec<String>, anyhow::Error> {
        let models = self.client.models().list().await?;
        Ok(models.data.into_iter().map(|x| x.id).collect())
    }

    pub async fn call_tool(&self, tool: ToolUse) -> Message {
        self.toolset.use_tool_async(tool).await.1
    }

    pub fn list_tools(&self) -> Vec<ToolDescription> {
        self.toolset.list_tools_to_human()
    }

    pub fn get_history_list(&self) -> Vec<ChatMeta> {
        self.history
            .iter()
            .filter_map(|v| v.ok())
            .filter_map(|(_, v)| serde_json::from_slice::<ChatMeta>(&v).ok())
            .collect()
    }

    pub fn get_chat(&self, chat_id: Uuid) -> Result<Option<ChatEntry>, Error> {
        match self.history.get(chat_id)? {
            Some(ivec) => Ok(serde_json::from_slice(&ivec)?),
            None => Ok(None),
        }
    }

    pub fn get_image(&self, image_id: Uuid) -> Result<Option<Vec<u8>>, Error> {
        match self.image.get(image_id)? {
            Some(ivec) => Ok(Some(ivec.to_vec())),
            None => Ok(None),
        }
    }

    pub fn get_asset(&self, asset_id: Uuid) -> Result<Option<Vec<u8>>, Error> {
        match self.image.get(asset_id)? {
            Some(ivec) => Ok(Some(ivec.to_vec())),
            None => Ok(None),
        }
    }

    pub fn delete_entry_with_blobs(&self, msg: &Message) {
        for content in msg.content.iter() {
            match content {
                MessageContent::ImageBin(_, img_id, _) | MessageContent::ImageRef(img_id, _) => {
                    if let Err(e) = self.image.release(img_id.clone()) {
                        tracing::error!("Failed to cleanup image {}: {}", img_id, e);
                    }
                }
                MessageContent::AssetRef(asset_id, _) => {
                    if let Err(e) = self.asset.release(asset_id.clone()) {
                        tracing::error!("Failed to cleanup asset {}: {}", asset_id, e);
                    }
                }
                _ => {}
            }
        }
    }

    pub fn delete_chat(&self, chat_id: Uuid) -> Result<(), Error> {
        if let Some(ivec) = self.history.remove(chat_id)? {
            if let Ok(entry) = serde_json::from_slice::<ChatEntry>(&ivec) {
                for msg in entry.messages {
                    self.delete_entry_with_blobs(&msg);
                }
            }
        }
        Ok(())
    }

    pub fn save_image(&self, binary: &[u8]) -> Result<Uuid, Error> {
        self.image.save(binary).map_err(|e| e.into())
    }

    pub fn save_asset(&self, binary: &[u8]) -> Result<Uuid, Error> {
        self.asset.save(binary).map_err(|e| e.into())
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

    pub async fn regenerate_at(
        &self,
        chat_id: Uuid,
        target_id: Uuid,
        llm_config: LLMConfig,
        cancel_token: CancellationToken,
    ) -> Result<impl Stream<Item = Result<ChatEvent, Error>>, Error> {
        self.truncate_chat_history(chat_id, target_id)?;
        self.stream_chat_response(chat_id, llm_config, cancel_token)
            .await
    }

    pub async fn edit_message_and_regenerate(
        &self,
        chat_id: Uuid,
        message_id: Uuid,
        new_content: Vec<MessageContent>,
        llm_config: LLMConfig,
        cancel_token: CancellationToken,
    ) -> Result<impl Stream<Item = Result<ChatEvent, Error>>, Error> {
        self.edit_and_truncate_history(chat_id, message_id, new_content)?;
        self.stream_chat_response(chat_id, llm_config, cancel_token)
            .await
    }

    fn edit_and_truncate_history(
        &self,
        chat_id: Uuid,
        target_id: Uuid,
        new_content: Vec<MessageContent>,
    ) -> Result<(), Error> {
        let mut entry = self
            .get_chat(chat_id)?
            .ok_or(anyhow!("Can not found chat {} from database.", chat_id))?;

        if let Some(index) = entry.messages.iter().position(|m| m.id == target_id) {
            if entry.messages[index].owner == Role::User {
                for msg in entry.messages.iter().skip(index + 1) {
                    self.delete_entry_with_blobs(msg);
                }
                entry.messages.truncate(index + 1);

                entry.messages[index].content = new_content;

                self.history
                    .insert(chat_id.as_bytes(), serde_json::to_vec(&entry)?)?;
                tracing::info!("Edited message {} and truncated history", target_id);
            } else {
                bail!("Edited message does not belong to user")
            }
        }
        Ok(())
    }

    pub async fn send_chat_message(
        &self,
        chat_id: Uuid,
        user_content: Vec<MessageContent>,
        llm_config: LLMConfig,
        cancel_token: CancellationToken,
    ) -> Result<impl Stream<Item = Result<ChatEvent, Error>>, Error> {
        let user_message = Message {
            id: Uuid::new_v4(),
            owner: Role::User,
            content: user_content,
            reasoning: vec![],
            tool_use: vec![],
        };
        self.append_message(chat_id, user_message)?;
        self.stream_chat_response(chat_id, llm_config, cancel_token)
            .await
    }

    async fn stream_chat_response(
        &self,
        chat_id: Uuid,
        llm_config: LLMConfig,
        cancel_token: CancellationToken,
    ) -> Result<impl Stream<Item = Result<ChatEvent, Error>>, Error> {
        let provider = self.clone();
        Ok(try_stream! {
            let mut current_session = provider.get_chat(chat_id)?.ok_or(anyhow!("Unexpected empty chat {}", chat_id))?;
            loop {
                let req_messages = provider.message_to_openai(current_session.clone(), llm_config.parallel_function_call.unwrap_or(false), llm_config.system_prompt_lang, llm_config.custom_system_prompt.clone());
                let mut req: CreateChatCompletionRequest = llm_config.clone().into();
                req.messages = req_messages;
                req.stream_options = Some(ChatCompletionStreamOptions{
                    include_usage: true
                });

                let chat = self.client.chat();
                let stream_future = chat.create_stream(req);
                let stream_result = tokio::select! {
                    res = stream_future => res,
                    _ = cancel_token.cancelled() => {
                        tracing::info!("Chat cancelled during stream creation");
                        return;
                    }
                };
                let mut stream = stream_result?;

                let mut state = StreamParseState::AwaitingDecision;
                            let mut parse_buffer: String = String::new(); // 切换回 String
                            let mut assistant_thinking = String::new();
                            let mut assistant_reasoning = String::new();
                            let mut assistant_content = String::new();
                            let mut assistant_tool_calls = Vec::new();
                            let mut current_tool_name = String::new();

                            while let Some(thunk) = stream.next().await {
                                let thunk = thunk?;
                                if let Some(usage) = thunk.usage {
                                        yield ChatEvent::Usage(usage);
                                }
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
                                'parse_loop: loop {
                                    match state {
                                        StreamParseState::AwaitingDecision => {
                                            if let Some(idx) = parse_buffer.find(FN_NAME) {
                                                let reasoning_part: String = parse_buffer.drain(..idx).collect();
                                                parse_buffer.drain(..FN_NAME.len());

                                                if !reasoning_part.is_empty() {
                                                    // 在第一次FNCALL之前，非主动think的时候
                                                    // 当作普通的文本输出
                                                    // 对于非thinking的model有必要
                                                    if assistant_reasoning.is_empty() {
                                                        assistant_content.push_str(&reasoning_part);
                                                        yield ChatEvent::ContentDelta(reasoning_part);
                                                    } else {
                                                        assistant_reasoning.push_str(&reasoning_part);
                                                        yield ChatEvent::ReasoningDelta(reasoning_part);
                                                    }
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

                                                if current_tool_name.len() == 0 {
                                                    tracing::warn!("ToolName is empty: {}, current ctx: {}", idx, content);
                                                }

                                                let delta = name_part_raw + FN_ARGS;
                                                assistant_reasoning.push_str(&delta);
                                                yield ChatEvent::ToolDelta(delta);

                                                state = StreamParseState::ToolCallArgs;
                                            } else {
                                                break 'parse_loop;
                                            }
                                        }
                                        StreamParseState::ToolCallArgs => {
                                            yield ChatEvent::ToolDelta(content.to_string());
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

                                                let tool_use =
                                                    ToolUse {
                                                        use_id: Uuid::new_v4(),
                                                        function_name: current_tool_name.clone(), args };
                                                yield ChatEvent::ToolCall(tool_use.clone());
                                                assistant_tool_calls.push(tool_use);

                                                //如果有连续的两个args，那就当作对同一个fn name使用2次
                                                //current_tool_name.clear();
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
                                }}
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

                        let tool_use = ToolUse {
                            use_id: Uuid::new_v4(),
                            function_name: current_tool_name.clone(),
                            args
                        };
                        yield ChatEvent::ToolCall(tool_use.clone());
                        assistant_tool_calls.push(tool_use);
                    }
                }
                let assistant_message = Message {
                        id: Uuid::new_v4(),
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
                    futures.push(provider.toolset.use_tool_async(tool_call.clone()));
                }

                let results: Vec<(ToolUse, Message)> = futures::future::join_all(futures).await;

                for (tool_use, res) in results.into_iter() {
                    yield ChatEvent::ToolResult { tool_use, result: res.clone() };
                    current_session = provider.append_message(chat_id, res)?;
                }

            }
        })
    }

    fn truncate_chat_history(&self, chat_id: Uuid, target_id: Uuid) -> Result<(), Error> {
        let mut entry = self
            .get_chat(chat_id)?
            .ok_or(anyhow!("Can not found chat {} from database.", chat_id))?;

        if let Some(index) = entry.messages.iter().position(|m| m.id == target_id) {
            let keep_count = if entry.messages[index].owner == Role::User {
                index + 1
            } else {
                index
            };

            for msg in entry.messages.iter().skip(keep_count) {
                self.delete_entry_with_blobs(msg);
            }
            entry.messages.truncate(keep_count);

            if entry.messages.is_empty() {
                bail!("Unexpected regeneration {} from starting", chat_id);
            }

            //TODO this should be replaced with compare_and_swap
            self.history
                .insert(chat_id.as_bytes(), serde_json::to_vec(&entry)?)?;
        } else {
            tracing::warn!("Target message {} not found in chat {}", target_id, chat_id);
        }
        Ok(())
    }

    #[allow(dead_code)]
    fn hydrate_image_ref(&self, content: &MessageContent) -> Result<MessageContent, Error> {
        match content {
            MessageContent::ImageRef(id, label) => {
                let image_data = self
                    .image
                    .get(id.clone())?
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
        lang: Option<whatlang::Lang>,
        custom_prompt: Option<String>,
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
                .unwrap_or(whatlang::Lang::Cmn),
        };
        tracing::debug!("User input language: {}", lang);
        let core_system_prompt = self.toolset.system_prompt(lang, is_parallel_fc);
        let final_system_prompt = if let Some(user_prompt) = custom_prompt {
            if !user_prompt.trim().is_empty() {
                format!(
                    "{}\n\n--- System Capabilities ---\n{}",
                    user_prompt, core_system_prompt
                )
            } else {
                core_system_prompt
            }
        } else {
            core_system_prompt
        };

        let system_message =
            ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
                content: ChatCompletionRequestSystemMessageContent::Text(final_system_prompt),
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
            Role::Tools(id) => ChatCompletionRequestMessage::Tool({
                //原始的async-openai不支持tool返回image，这里用的是修改版
                //let mut r = ChatCompletionRequestUserMessage::default();
                //r.content = ChatCompletionRequestUserMessageContent::Array(
                //    self.map_multi_modal_tool_messages(v)?,
                //);
                let mut r = ChatCompletionRequestToolMessage::default();
                r.tool_call_id = id.to_string();
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
                MessageContent::AssetRef(_, _) => {
                    // Asset返回描述文字
                    res.push(ChatCompletionRequestToolMessageContentPart::Text(
                        ChatCompletionRequestMessageContentPartText {
                            text: msg.to_string(),
                        },
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
                            tracing::warn!(
                                "Unable to get tool result image {} from database: {}, skip.",
                                id,
                                e
                            )
                        }
                        Ok(None) => {
                            tracing::warn!("Tool result image {} not in database, skip.", id)
                        }
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
                MessageContent::AssetRef(_, _) => {
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
                    format!("[IMG]")
                }
                MessageContent::AssetRef(_, _) => {
                    format!("[BIN]")
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
