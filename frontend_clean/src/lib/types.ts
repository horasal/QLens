export type ChatMeta = {
    id: string;
    date: string;
    summary: string;
};

export type CompletionUsage = {
    prompt_tokens: number;
    completion_tokens: number;
    total_tokens: number;
};

export type ToolDescription = {
  name_for_model: string;
  name_for_human: string;
  description_for_model: string;
  parameters: any;
  args_format: string;
};

// 对应 Rust enum MessageContent
export type MessageContent =
    | { Text: string }
    | { ImageRef: [string, string] } // [uuid, description]
    | { ImageBin: [string, string, string] } // [base64, uuid, description]
    | { AssetRef: [string, string] }; // [uuid, description]

export type ToolUse = {
    use_id: string;
    function_name: string;
    args: string;
};

export type Role =
    | { role: 'user' }
    | { role: 'assistant' }
    | { role: 'system' }
    | { role: 'tool'; tool_call_id: string };

export type Message = {
    id: string;
    owner: Role;
    reasoning: MessageContent[];
    content: MessageContent[];
    tool_use: ToolUse[];
    tool_deltas?: string;
};

export type ClientRequest =
    | { type: 'Chat'; payload: { request_id: string; chat_id: string; content: MessageContent[], config?: any } }
    | { type: 'Abort'; payload: { request_id: string; chat_id: string } }
    | { type: 'Regenerate'; payload: { request_id: string; chat_id: string; message_id: string, config?: any } }
    | { type: 'Edit'; payload: { request_id: string; chat_id: string; message_id: string; new_content: MessageContent[]; config?: any } };

export type ChatEntry = {
    id: string;
    date: string;
    summary: string;
    messages: Message[];
};

// WebSocket 消息类型
export type ClientWSMessage = {
    chat_id: string;
    content: MessageContent[];
};

export interface StreamPacket {
    chat_id: string;
    request_id: string;
    // 这里的 key 对应 Rust ChatEvent 的字段
    ReasoningDelta?: string;
    ToolDelta?: string;
    ToolCall?: ToolUse;
    ToolResult?: { tool_use: ToolUse; result: Message };
    ContentDelta?: string;
    StreamEnd?: boolean;
    Usage?: CompletionUsage;
    Error?: string;
}

export type UploadImageResponse = {
    file: string;
    uuid: string;
};

// Toast 通知类型
export type ToastType = 'error' | 'info' | 'success';
export type Toast = {
    id: number;
    message: string;
    type: ToastType;
};

export type PreviewFile = {
    url: string;
    file: File;
};

export type PendingFile =
    | { type: 'image'; file: File; url: string; id: string } // 图片：需预览 + 上传
    | { type: 'text_content'; file: File; content: string; id: string } // 小文本：直接作为 Text 发送
    | { type: 'asset'; file: File; id: string }; // 大文件/其他：需上传作为 AssetRef
