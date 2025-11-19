export type ChatMeta = {
    id: string;
    date: string;
    summary: string;
};

// 对应 Rust enum MessageContent
export type MessageContent =
    | { Text: string }
    | { ImageRef: [string, string] } // [uuid, description]
    | { ImageBin: [string, string, string] }; // [base64, uuid, description]

export type ToolUse = {
    function_name: string;
    args: string;
};
export type Message = {
    id: string; // <--- 新增：对应后端的 Uuid
    owner: 'User' | 'Assistant' | 'System' | 'Tools';
    reasoning: MessageContent[];
    content: MessageContent[];
    tool_use: ToolUse[];
    tool_deltas?: string;
};

export type ClientRequest =
    | { type: 'Chat'; payload: { request_id: string; chat_id: string; content: MessageContent[] } }
    | { type: 'Abort'; payload: { request_id: string; chat_id: string } }
    | { type: 'Regenerate'; payload: { request_id: string; chat_id: string; message_id: string } };

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
    ToolResult?: { tool_use: ToolUse; result: MessageContent[] };
    ContentDelta?: string;
    StreamEnd?: boolean;
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
