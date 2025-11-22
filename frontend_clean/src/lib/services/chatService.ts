import { get } from 'svelte/store';
import { goto } from '$app/navigation';
import {
	currentChat,
	historyList,
	processingChatIds,
	wipDeltaStore,
	isLoading,
	toasts,
	connectionStatus
} from '../stores/chatStore';
import type {
	ChatEntry,
	ChatMeta,
	ClientRequest,
	Message,
	MessageContent,
	StreamPacket,
	UploadImageResponse,
	PreviewFile,
	ToolDescription
} from '../types';
import { settings } from '$lib/stores/settingsStore';
import { currentUsage } from '../stores/chatStore';
import { randomUUID } from 'crypto';

// --- 局部状态 (不需要 UI 响应的) ---
let ws: WebSocket | null = null;
let reconnectInterval = 1000;
const maxReconnectInterval = 30000;
let wsReconnecting = false;
// 记录 chat_id -> request_id 的映射，用于发送中止请求
const activeRequestIds = new Map<string, string>();

export async function init() {
	await loadHistorySidebar();
	connectWebSocket();
}

export async function regenerateMessage(messageId: string) {
	const curr = get(currentChat);
	if (!curr || !ws) return;

	const requestId = self.crypto.randomUUID();
	processingChatIds.add(curr.id);
	activeRequestIds.set(curr.id, requestId);
	currentChat.update((chat) => {
		if (!chat) return null;

		const index = chat.messages.findIndex((m) => m.id === messageId);
		if (index !== -1) {
			const targetMsg = chat.messages[index];

			if (targetMsg.owner.role === 'assistant' || targetMsg.owner.role === 'tool') {
				chat.messages = chat.messages.slice(0, index);
			} else {
				chat.messages = chat.messages.slice(0, index + 1);
			}
		}
		return chat;
	});

	try {
		if (!(await waitForConnection())) throw new Error('Connection timeout');

		const payload: ClientRequest = {
			type: 'Regenerate',
			payload: {
				request_id: requestId,
				chat_id: curr.id,
				message_id: messageId,
				config: settings.getLLMConfig()
			}
		};
		ws.send(JSON.stringify(payload));
	} catch (e: any) {
		toasts.show(e.message);
		processingChatIds.delete(curr.id);
	}
}

export async function editMessage(messageId: string, newText: string) {
	const curr = get(currentChat);
	const currentSettings = get(settings); // 获取当前配置
	if (!curr || !ws) return;

	const requestId = self.crypto.randomUUID();
	processingChatIds.add(curr.id);
	activeRequestIds.set(curr.id, requestId); // 记录以便 Abort

	currentChat.update((chat) => {
		if (!chat) return null;
		const index = chat.messages.findIndex((m) => m.id === messageId);
		if (index !== -1) {
			const oldImages = chat.messages[index].content.filter(
				(c) => 'ImageRef' in c || 'ImageBin' in c
			);
			const newContent: MessageContent[] = [...oldImages, { Text: newText }];

			chat.messages[index].content = newContent;

			chat.messages = chat.messages.slice(0, index + 1);
		}
		return chat;
	});

	try {
		const msg = curr.messages.find((m) => m.id === messageId);
		const oldImages = msg ? msg.content.filter((c) => 'ImageRef' in c) : [];
		const payloadContent = [...oldImages, { Text: newText }];

		const payload: ClientRequest = {
			type: 'Edit',
			payload: {
				request_id: requestId,
				chat_id: curr.id,
				message_id: messageId,
				new_content: payloadContent,
				config: settings.getLLMConfig()
			}
		};
		ws.send(JSON.stringify(payload));
	} catch (e: any) {
		toasts.show(e.message);
		processingChatIds.delete(curr.id);
	}
}

// --- WebSocket 连接逻辑 ---
function connectWebSocket() {
	if (ws && (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)) return;

	const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
	const host = window.location.host;
	const wsUrl = `${protocol}//${host}/api/chat`;

	console.log('Connecting to WebSocket at:', wsUrl);
	ws = new WebSocket(wsUrl);
	connectionStatus.set('reconnecting');

	ws.onopen = () => {
		console.log('WebSocket connected');
		connectionStatus.set('connected');
		wsReconnecting = false;
		reconnectInterval = 1000;

		// 如果当前有打开的对话，且没在生成中，尝试重新加载以同步最新状态
		const curr = get(currentChat);
		if (curr && !get(processingChatIds).has(curr.id)) {
			loadChat(curr.id);
		}
	};

	ws.onclose = (e) => {
		console.log('WebSocket disconnected', e);
		connectionStatus.set('disconnected');
		if (!wsReconnecting) {
			scheduleReconnect();
		}
	};

	ws.onerror = (err) => {
		console.error('WebSocket error', err);
		ws?.close();
	};

	ws.onmessage = handleWsMessage;
}

function scheduleReconnect() {
	wsReconnecting = true;
	toasts.show(
		`Connection lost. Reconnecting in ${reconnectInterval}ms...`,
		'info',
		reconnectInterval
	);
	setTimeout(() => {
		connectWebSocket();
		reconnectInterval = Math.min(reconnectInterval * 1.5, maxReconnectInterval);
	}, reconnectInterval);
}

async function waitForConnection(timeout = 5000): Promise<boolean> {
	if (ws?.readyState === WebSocket.OPEN) return true;
	connectWebSocket();
	const start = Date.now();
	while (ws?.readyState !== WebSocket.OPEN) {
		if (Date.now() - start > timeout) return false;
		await new Promise((r) => setTimeout(r, 100));
	}
	return true;
}

// --- WebSocket 消息处理 (路由) ---
function handleWsMessage(event: MessageEvent) {
	let packet: StreamPacket;
	try {
		packet = JSON.parse(event.data);
	} catch (e) {
		console.error('Failed to parse WS message', e);
		return;
	}

	const { chat_id } = packet;
	if (!chat_id) return;

	if (packet.Error) {
		console.error('Server Error:', packet.Error);
		toasts.show(`Server Error: ${packet.Error}`, 'error', 5000);
		processingChatIds.delete(chat_id);
		return;
	}

	if (packet.Usage) {
      currentUsage.set(packet.Usage);
      return;
  }

	// 1. 处理流结束信号
	if (packet.StreamEnd) {
		console.log(`StreamEnd received for: ${chat_id}`);
		processingChatIds.delete(chat_id);

		// 清理增量缓存
		wipDeltaStore.update((map) => {
			map.delete(chat_id);
			return map;
		});

		const curr = get(currentChat);
		if (chat_id === curr?.id) {
			// 刷新当前对话，确保完全同步
			loadChat(curr.id);
		} else {
			// 后台任务结束，刷新侧边栏
			loadHistorySidebar();
		}
		return;
	}

	// 2. 存储增量数据 (如果是在后台运行)
	if (get(processingChatIds).has(chat_id)) {
		wipDeltaStore.update((map) => {
			const list = map.get(chat_id) || [];
			list.push(packet);
			map.set(chat_id, list);
			return map;
		});
	}

	// 3. 如果是当前对话，实时更新 UI
	const curr = get(currentChat);
	if (chat_id === curr?.id) {
		currentChat.update((chat) => {
			if (!chat) return null;
			chat.messages = applyPacketToMessages(chat.messages, packet);
			return chat;
		});
	}
}

// --- API 操作 ---

export async function loadHistorySidebar() {
	try {
		const res = await fetch('/api/history');
		if (res.ok) {
			const chats: ChatMeta[] = await res.json();
			chats.sort((a, b) => Date.parse(b.date) - Date.parse(a.date));
			historyList.set(chats);
		} else {
			throw new Error('Failed to fetch history list');
		}
	} catch (e: any) {
		toasts.show(e.message || 'Error loading history');
	}
}

export async function loadChat(id: string) {
	isLoading.set(true);
	// 更新 URL 但不跳转页面
	goto(`?id=${id}`, { keepFocus: true, noScroll: true, replaceState: true });

	try {
		const res = await fetch(`/api/history/${id}`);
		if (res.ok) {
			let loadedChat: ChatEntry = await res.json();

			// 重放这一瞬间可能产生的增量包 (Replay WIP Deltas)
			const wipMap = get(wipDeltaStore);
			const deltas = wipMap.get(id);
			if (deltas) {
				console.log(`Replaying ${deltas.length} deltas...`);
				for (const p of deltas) {
					loadedChat.messages = applyPacketToMessages(loadedChat.messages, p);
				}
			}

			currentChat.set(loadedChat);
		} else {
			toasts.show('Chat not found', 'error');
			currentChat.set(null);
			goto('?', { replaceState: true });
		}
	} catch (e: any) {
		toasts.show(e.message, 'error');
	} finally {
		isLoading.set(false);
	}
}

export async function startNewChat() {
	isLoading.set(true);
	try {
		const res = await fetch('/api/chat/new', { method: 'POST' });
		if (!res.ok) throw new Error('Failed to create chat');

		const newChat: ChatEntry = await res.json();

		// 更新列表和当前视图
		historyList.update((l) => [
			{ id: newChat.id, date: newChat.date, summary: newChat.summary },
			...l
		]);
		currentChat.set(newChat);
		goto(`?id=${newChat.id}`, { replaceState: true });
	} catch (e: any) {
		toasts.show(e.message);
	} finally {
		isLoading.set(false);
	}
}

export async function deleteChat(id: string) {
	// 先停止生成
	abortGeneration(id);

	// 乐观更新 UI
	historyList.update((l) => l.filter((c) => c.id !== id));
	const curr = get(currentChat);
	if (curr?.id === id) {
		currentChat.set(null);
		goto('?', { replaceState: true });
	}
	processingChatIds.delete(id);
	wipDeltaStore.update((m) => {
		m.delete(id);
		return m;
	});

	try {
		const res = await fetch(`/api/history/${id}`, { method: 'DELETE' });
		if (!res.ok) throw new Error('Delete failed');
	} catch (e: any) {
		toasts.show(e.message);
		loadHistorySidebar(); // 失败回滚：重新加载列表
	}
}

export async function abortGeneration(chatId: string) {
	const reqId = activeRequestIds.get(chatId);
	if (!reqId || !ws) return;

	const payload: ClientRequest = {
		type: 'Abort',
		payload: { request_id: reqId, chat_id: chatId }
	};
	ws.send(JSON.stringify(payload));

	// 清理状态
	processingChatIds.delete(chatId);
	activeRequestIds.delete(chatId);
	toasts.show('Generation stopped', 'info');
}

export async function sendMessage(text: string, files: PreviewFile[]) {
	const curr = get(currentChat) || (await startNewChat().then(() => get(currentChat)));
	if (!curr) return;
  currentUsage.set(null);

	processingChatIds.add(curr.id);
	const requestId = self.crypto.randomUUID();
	activeRequestIds.set(curr.id, requestId);

	try {
		let imageRefs: MessageContent[] = [];
		if (files.length > 0) {
			const formData = new FormData();
			files.forEach((f) => formData.append('files', f.file, f.file.name));

			const res = await fetch('/api/image', { method: 'POST', body: formData });
			if (!res.ok) throw new Error('Image upload failed');

			const uploaded: UploadImageResponse[] = await res.json();
			imageRefs = uploaded.map((r) => ({ ImageRef: [r.uuid, r.file] }));
		}

		const textContent: MessageContent[] = text.trim() ? [{ Text: text.trim() }] : [];
		const fullContent = [...imageRefs, ...textContent];

		const userMsg: Message = {
  		id: crypto.randomUUID(),
      owner: { role: 'user' },
			reasoning: [],
			content: fullContent,
			tool_use: []
		};

		currentChat.update((c) => {
			if (c) c.messages = [...c.messages, userMsg];
			return c;
		});

		if (!(await waitForConnection())) throw new Error('Connection timeout');

		const payload: ClientRequest = {
			type: 'Chat',
			payload: {
				request_id: requestId,
				chat_id: curr.id,
				content: fullContent,
				config: settings.getLLMConfig()
			}
		};
		ws?.send(JSON.stringify(payload));
	} catch (e: any) {
		toasts.show(e.message);
		processingChatIds.delete(curr.id);
		activeRequestIds.delete(curr.id);
	}
}

function appendTextDelta(contentArray: MessageContent[], delta: string) {
	if (contentArray.length > 0 && 'Text' in contentArray[contentArray.length - 1]) {
		// @ts-ignore: TS sometimes struggles with discriminated union access
		contentArray[contentArray.length - 1].Text += delta;
	} else {
		contentArray.push({ Text: delta });
	}
}

function applyPacketToMessages(messages: Message[], packet: StreamPacket): Message[] {
	// 辅助：获取或创建当前正在生成的 Assistant 消息
	const getOrCreateWipAssistant = (): Message => {
		let last = messages[messages.length - 1];
		if (last && last.owner.role === 'assistant') {
			if (last.tool_deltas === undefined) last.tool_deltas = '';
			return last;
		}
		const newMsg: Message = {
  		id: crypto.randomUUID(),
      owner: { role: 'assistant' } ,
			reasoning: [],
			content: [],
			tool_use: [],
			tool_deltas: ''
		};
		messages.push(newMsg);
		return newMsg;
	};

	if (packet.ReasoningDelta) {
		appendTextDelta(getOrCreateWipAssistant().reasoning, packet.ReasoningDelta);
	} else if (packet.ToolDelta) {
		getOrCreateWipAssistant().tool_deltas += packet.ToolDelta;
	} else if (packet.ToolCall) {
		const msg = getOrCreateWipAssistant();
		msg.tool_use.push(packet.ToolCall);
		msg.tool_deltas = ''; // Reset deltas after a full call is parsed
	} else if (packet.ToolResult) {
			messages.push(packet.ToolResult.result);
	} else if (packet.ContentDelta) {
		appendTextDelta(getOrCreateWipAssistant().content, packet.ContentDelta);
	}

	return messages;
}

export async function getTools(): Promise<ToolDescription[]> {
    try {
        const res = await fetch('/api/tools');
        if (!res.ok) throw new Error('Failed to fetch tools');
        return await res.json();
    } catch (e) {
        console.error(e);
        return [];
    }
}

export async function runTool(name: string, args: string): Promise<Message | null> {
    try {
        const res = await fetch(`/api/tools/${name}`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ args })
        });
        if (!res.ok) throw new Error(`Tool execution failed: ${res.statusText}`);
        return await res.json();
    } catch (e: any) {
        toasts.show(e.message, 'error');
        return null;
    }
}
