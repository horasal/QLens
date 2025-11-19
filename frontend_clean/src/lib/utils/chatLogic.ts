import type { Message, MessageContent } from '../types';

function appendTextDelta(contentArray: MessageContent[], delta: string) {
	if (contentArray.length > 0 && 'Text' in contentArray[contentArray.length - 1]) {
		contentArray[contentArray.length - 1].Text += delta;
	} else {
		// 还没有 Text 元素，或最后一个不是 Text，创建新元素
		contentArray.push({ Text: delta });
	}
}

function applyPacketToMessages(messages: Message[], packet: any): Message[] {
	const getOrCreateWipAssistant = (): Message => {
		let lastMessage = messages[messages.length - 1];
		if (lastMessage && lastMessage.owner === 'Assistant') {
			if (lastMessage.tool_deltas === undefined) lastMessage.tool_deltas = '';
			return lastMessage;
		}
		const newWipMessage: Message = {
			owner: 'Assistant',
			reasoning: [],
			content: [],
			tool_use: [],
			tool_deltas: ''
		};
		messages.push(newWipMessage);
		return newWipMessage;
	};

	if (packet.ReasoningDelta) {
		const wip = getOrCreateWipAssistant();
		appendTextDelta(wip.reasoning, packet.ReasoningDelta);
	} else if (packet.ToolDelta) {
		const wip = getOrCreateWipAssistant();
		wip.tool_deltas += packet.ToolDelta;
	} else if (packet.ToolCall) {
		const wip = getOrCreateWipAssistant();
		wip.tool_use.push(packet.ToolCall);
		wip.tool_deltas = '';
	} else if (packet.ToolResult) {
		let lastMessage = messages[messages.length - 1];
		if (lastMessage && lastMessage.owner === 'Tools') {
			lastMessage.content.push(...packet.ToolResult.result);
		} else {
			const toolMessage: Message = {
				owner: 'Tools',
				reasoning: [],
				content: packet.ToolResult.result,
				tool_use: []
			};
			messages.push(toolMessage);
		}
	} else if (packet.ContentDelta) {
		const wip = getOrCreateWipAssistant();
		appendTextDelta(wip.content, packet.ContentDelta);
	}

	return messages; // 返回修改后的数组
}
