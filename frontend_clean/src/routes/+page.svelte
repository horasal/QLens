<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { renderMarkdown } from '../lib/markdownRenderer';
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import { resolve } from '$app/paths';
	import {
		_,
		register,
		init,
		getLocaleFromNavigator,
		isLoading as i18n_loading
	} from 'svelte-i18n';

	register('en', () => import('../langs/en.json'));
	register('zh', () => import('../langs/zh.json'));
	register('zh-CN', () => import('../langs/zh.json'));
	register('ko', () => import('../langs/ko.json'));
	register('ja', () => import('../langs/ja.json'));
	init({
		fallbackLocale: 'en',
		initialLocale: getLocaleFromNavigator()
	});

	type ChatMeta = {
		id: string;
		date: string;
		summary: string;
	};

	// 客户端 WebSocket 消息 (对应 Rust 的 ClientWSMessage)
	type ClientWSMessage = {
		chat_id: string;
		content: MessageContent[];
	};

	type MessageContent =
		| { Text: string }
		| { ImageRef: [string, string] } // [uuid, description]
		| { ImageBin: [string, string, string] }; // [base64, uuid, description] - (用于流式)

	type ToolUse = {
		function_name: string;
		args: string;
	};

	type Message = {
		owner: 'User' | 'Assistant' | 'System' | 'Tools';
		reasoning: MessageContent[];
		content: MessageContent[];
		tool_use: ToolUse[];
		tool_deltas?: string;
	};

	type ChatEntry = {
		id: string;
		date: string;
		summary: string;
		messages: Message[];
	};

	// Rust 返回的图片上传响应
	type UploadImageResponse = {
		file: string;
		uuid: string;
	};

	// --- 状态变量 ---
	let historyList: ChatMeta[] = [];
	let currentChat: ChatEntry | null = null;
	let modalImageUrl: string | null = null;
	let imageModal: HTMLDialogElement;

	let textInput: string = '';
	let isLoading: boolean = false;
	let processingChatIds = new Set<string>();
	let ws: WebSocket;

	let previewFiles: { url: string; file: File }[] = [];
	let isDragging = false;
	let errorMessage = '';
	let wipDeltaStore = new Map<string, any[]>();
	let errorTimeoutId: number | null = null;

	let autoScrollEnabled = true;
	let showScrollToBottomButton = false;
	let chatContainer: HTMLElement;
	let mainContentArea: HTMLElement;

	function showErrorToast(message: string, duration_ms = 5000) {
		errorMessage = message;

		// 如果有旧的计时器，清除它
		if (errorTimeoutId) {
			clearTimeout(errorTimeoutId);
		}

		errorTimeoutId = setTimeout(() => {
			errorMessage = '';
			errorTimeoutId = null;
		}, duration_ms);
	}

	async function deleteChat(id: string) {
		historyList = historyList.filter((chat) => chat.id !== id);

		if (currentChat?.id === id) {
			currentChat = null;
			goto('?', { replaceState: true });
		}

		processingChatIds.delete(id);
		processingChatIds = processingChatIds;
		wipDeltaStore.delete(id);

		try {
			const response = await fetch(`/api/history/${id}`, {
				method: 'DELETE'
			});

			if (!response.ok) {
				// 如果失败，显示错误并...重新加载侧边栏以恢复？
				const errorText = await response.text();
				throw new Error(`Failed to delete chat: ${errorText}`);
			}
			console.log(`Chat ${id} deleted successfully.`);
		} catch (err) {
			console.error('Delete chat error:', err);
			showErrorToast(err.message);
			// 失败时从服务器完全重新同步
			loadHistorySidebar();
		}
	}

	async function startNewChat() {
		isLoading = true;
		try {
			const res = await fetch('/api/chat/new', { method: 'POST' });

			if (!res.ok) {
				throw new Error(`Failed to create new chat: ${res.statusText}`);
			}

			const newChat: ChatEntry = await res.json();

			historyList = [
				{ id: newChat.id, date: newChat.date, summary: newChat.summary },
				...historyList
			];

			currentChat = newChat;
			goto('?id=' + newChat.id, { replaceState: true });
		} catch (err) {
			console.error('Failed to start new chat:', err);
			showErrorToast(err.message);
		}
		isLoading = false;
	}
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

	onMount(async () => {
		await loadHistorySidebar();

		// 动态构建 WebSocket URL
		const wsProtocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
		const wsUrl = `${wsProtocol}//${window.location.host}/api/chat`;

		console.log('Connecting to WebSocket at:', wsUrl);
		ws = new WebSocket(wsUrl);

		ws.onopen = () => console.log('WebSocket connected');
		ws.onclose = () => console.log('WebSocket disconnected');

		ws.onmessage = (event) => {
			const packet = JSON.parse(event.data);

			const packetChatId = packet.chat_id;
			if (!packetChatId) {
				console.error('Received packet without chat_id:', packet);
				return;
			}

			if (packet.StreamEnd) {
				console.log(`StreamEnd received for: ${packetChatId}`);

				processingChatIds.delete(packetChatId);
				processingChatIds = processingChatIds; // 强制响应式

				wipDeltaStore.delete(packetChatId);

				if (packetChatId === currentChat?.id) {
					console.log(`Reloading current chat ${currentChat.id} from database...`);
					loadChat(currentChat.id);
				} else {
					console.log(`Reloading sidebar, background chat ${packetChatId} finished.`);
					loadHistorySidebar();
				}
				return;
			}

			if (processingChatIds.has(packetChatId)) {
				if (!wipDeltaStore.has(packetChatId)) {
					wipDeltaStore.set(packetChatId, []);
				}
				wipDeltaStore.get(packetChatId).push(packet);
			}
			if (packetChatId === currentChat?.id) {
				currentChat.messages = applyPacketToMessages(currentChat.messages, packet);
				currentChat = currentChat; // 强制响应式

				scrollToBottom();
			} else if (processingChatIds.has(packetChatId)) {
				console.log(`Stored packet for background chat: ${packetChatId}`);
			} else {
				console.log(`Ignoring packet for background chat: ${packetChatId}`);
			}
		};
		const urlId = $page.url.searchParams.get('id');
		if (urlId) {
			await loadChat(urlId);
		}
	});

	async function loadHistorySidebar() {
		try {
			// 使用相对路径
			const response = await fetch('/api/history');
			if (response.ok) {
				const chats: ChatMeta[] = await response.json();
				chats.sort((a, b) => Date.parse(b.date) - Date.parse(a.date));
				historyList = chats;
			} else {
				console.error('Failed to fetch history list');
				showErrorToast('Failed to fetch history list');
			}
		} catch (e) {
			console.error('Error fetching history:', e);
			showErrorToast('Error fetching history:' + e);
		}
	}

	async function loadChat(id: string) {
		isLoading = true;
		currentChat = null;
		goto('?id=' + id, { keepFocus: true, noScroll: true, replaceState: true });
		try {
			const response = await fetch(`/api/history/${id}`);
			if (response.ok) {
				let loadedChat: ChatEntry = await response.json();
				const deltas = wipDeltaStore.get(id);

				if (deltas) {
					console.log(`Replaying ${deltas.length} stored deltas for chat ${id}...`);
					let messages = loadedChat.messages;
					for (const packet of deltas) {
						messages = applyPacketToMessages(messages, packet);
					}
					loadedChat.messages = messages;
				}
				currentChat = loadedChat;
				// 自动滚动到底部
				setTimeout(() => scrollToBottom(), 0);
			} else {
				console.error('Failed to fetch chat entry');
				showErrorToast('Failed to fetch chat entry list');
				currentChat = null;
				goto('?', { replaceState: true });
			}
		} catch (e) {
			console.error('Error fetching chat:', e);
			showErrorToast('Error fetching chat:' + e);
			goto('?', { replaceState: true });
		}
		isLoading = false;
	}

	function getImageUrl(item: MessageContent): string | null {
		if ('ImageRef' in item) {
			// [uuid, description]
			// 从服务器加载持久化图片
			return `/api/image/${item.ImageRef[0]}`; // 使用相对路径
		}
		if ('ImageBin' in item) {
			// [base64, uuid, description]
			return `data:image/jpeg;base64,${item.ImageBin[0]}`;
		}
		return null;
	}

	function showImageModal(src: string) {
		modalImageUrl = src;
		imageModal?.showModal();
	}

	function formatToolArgs(args: string): string {
		try {
			const parsed = JSON.parse(args);
			return JSON.stringify(parsed, null, 2);
		} catch {
			return args; // 如果不是 JSON，则按原样返回
		}
	}

	function handleMarkdownClick(event: MouseEvent) {
		// 检查被点击的目标是否是一个 <img> 标签
		const target = event.target as HTMLElement;
		if (target.tagName === 'IMG') {
			const src = target.getAttribute('src');
			if (src) {
				showImageModal(src);
			}
		}
	}

	async function sendMessage() {
		if (
			processingChatIds.has(currentChat.id) ||
			!currentChat ||
			(textInput.trim() === '' && previewFiles.length === 0)
		) {
			return;
		}

		processingChatIds.add(currentChat.id);
		processingChatIds = processingChatIds;

		let imageRefs: MessageContent[] = [];

		if (previewFiles.length > 0) {
			const formData = new FormData();
			for (const item of previewFiles) {
				formData.append('files', item.file, item.file.name);
			}

			try {
				const res = await fetch('/api/image', {
					method: 'POST',
					body: formData
				});

				if (!res.ok) throw new Error(`File upload failed: ${res.statusText}`);

				const uploadResponses: UploadImageResponse[] = await res.json();
				imageRefs = uploadResponses.map((resp) => ({
					ImageRef: [resp.uuid, resp.file]
				}));
			} catch (err) {
				console.error('Upload error:', err);
				processingChatIds.delete(currentChat.id);
				processingChatIds = processingChatIds;
				showErrorToast(err.message);
				return;
			}
		}

		const textContent: MessageContent[] = [];
		if (textInput.trim() !== '') {
			textContent.push({ Text: textInput.trim() });
		}

		const fullUserContent = [...imageRefs, ...textContent];

		// 将用户消息添加到 UI
		const userMessage: Message = {
			owner: 'User',
			reasoning: [],
			content: fullUserContent,
			tool_use: []
		};

		currentChat.messages = [...currentChat.messages, userMessage];
		currentChat = currentChat; // 触发 Svelte 响应

		const wsMessage: ClientWSMessage = {
			chat_id: currentChat.id,
			content: fullUserContent
		};

		ws.send(JSON.stringify(wsMessage));

		textInput = '';
		previewFiles = [];

		setTimeout(() => scrollToBottom(true), 0);
	}

	function handleScroll() {
		if (!chatContainer) return;

		const threshold = 15; // 距离底部的容差 (px)
		const isNearBottom =
			chatContainer.scrollHeight - chatContainer.scrollTop - chatContainer.clientHeight < threshold;

		if (isNearBottom) {
			// 用户滚动回了底部
			autoScrollEnabled = true;
			showScrollToBottomButton = false;
		} else {
			// 用户向上滚动了
			autoScrollEnabled = false;
			showScrollToBottomButton = true;
		}
	}
	function handleFileSelect(files: FileList | null) {
		if (!files) return;

		for (const file of files) {
			if (!file.type.startsWith('image/')) continue;

			const reader = new FileReader();
			reader.onload = (e) => {
				const url = e.target?.result as string;
				previewFiles = [...previewFiles, { url, file }];
			};
			reader.readAsDataURL(file);
		}
	}

	function removePreview(index: number) {
		previewFiles = previewFiles.filter((_, i) => i !== index);
	}

	function handleDragOver(e: DragEvent) {
		e.preventDefault();
		isDragging = true;
	}

	function handleDragLeave(e: DragEvent) {
		if (e.relatedTarget === null || !mainContentArea.contains(e.relatedTarget as Node)) {
			isDragging = false;
		}
	}

	function handleDrop(e: DragEvent) {
		e.preventDefault();
		isDragging = false;
		handleFileSelect(e.dataTransfer?.files ?? null);
	}

	async function scrollToBottom(force = false) {
		if (!autoScrollEnabled && !force) {
			return;
		}
		if (force) {
			autoScrollEnabled = true;
			showScrollToBottomButton = false;
		}

		await tick();
		const el = document.getElementById('chat-container-end');
		el?.scrollIntoView({ behavior: 'smooth' });
	}
</script>

{#if $i18n_loading}
	<div
		class="fixed inset-0 z-50 flex flex-col items-center justify-center bg-base-100 text-primary"
	>
		<span class="loading loading-lg scale-150 loading-infinity"></span>

		<p class="mt-6 animate-pulse text-sm font-medium tracking-widest uppercase opacity-70">
			UI Loading
		</p>
	</div>
{:else}
	<dialog bind:this={imageModal} class="modal">
		<div class="modal-box max-w-5xl p-0">
			<img src={modalImageUrl} alt="Enlarged view" class="w-full" />
		</div>
		<form method="dialog" class="modal-backdrop">
			<button>close</button>
		</form>
	</dialog>

	<div class="drawer lg:drawer-open" data-theme="winter">
		<input id="my-drawer" type="checkbox" class="drawer-toggle" />

		<div class="drawer-side">
			<label for="my-drawer" aria-label="close sidebar" class="drawer-overlay"></label>
			<ul class="menu min-h-full w-80 overflow-y-auto bg-base-200 p-4 text-base-content">
				<li class="mb-2 w-full min-w-1">
					<button class="btn btn-primary" on:click={startNewChat}> + {$_('new_chat')} </button>
				</li>
				{#each historyList as chat (chat.id)}
					<li class:active={currentChat?.id === chat.id} class="ovelflow-hidden w-full">
						<a
							on:click={() => {
								loadChat(chat.id);
							}}
							class="group flex w-full items-center justify-between"
						>
							<button
								class="btn btn-circle text-error/70 opacity-0 btn-ghost transition-opacity btn-xs group-hover:opacity-100 hover:bg-error/20"
								on:click|stopPropagation={() => deleteChat(chat.id)}
							>
								✕
							</button>

							<div class="mx-2 min-w-0 flex-1 overflow-hidden">
								<p class="truncate">{chat.summary || $_('no_title')}</p>
								<span class="text-xs text-base-content/50">
									{new Date(chat.date).toLocaleString()}
								</span>
							</div>

							{#if processingChatIds.has(chat.id)}
								<span class="loading ml-2 loading-xs loading-spinner text-primary"></span>
							{/if}
						</a>
					</li>
				{/each}
			</ul>
		</div>

		<div
			class="relative drawer-content flex h-screen flex-col"
			bind:this={mainContentArea}
			on:dragover={handleDragOver}
			on:dragleave={handleDragLeave}
			on:drop={handleDrop}
		>
			<div class="toast toast-start toast-bottom z-50">
				{#if errorMessage}
					<div class="alert alert-error shadow-lg">
						<span>{errorMessage}</span>
					</div>
				{/if}
			</div>
			<div class="navbar bg-base-100 lg:hidden">
				<div class="flex-none">
					<label for="my-drawer" class="btn btn-square btn-ghost">
						<svg
							xmlns="http://www.w3.org/2000/svg"
							fill="none"
							viewBox="0 0 24 24"
							class="inline-block h-5 w-5 stroke-current"
							><path
								stroke-linecap="round"
								stroke-linejoin="round"
								stroke-width="2"
								d="M4 6h16M4 12h16M4 18h16"
							></path></svg
						>
					</label>
				</div>
				<div class="flex-1">
					<a class="btn text-xl btn-ghost">QLens</a>
				</div>
			</div>

			<div
				class="flex-1 overflow-y-auto bg-base-100 p-4"
				bind:this={chatContainer}
				on:scroll={handleScroll}
			>
				{#if isLoading}
					<div class="flex h-full items-center justify-center">
						<span class="loading loading-lg loading-spinner text-primary"></span>
					</div>
				{:else if !currentChat}
					<div class="flex h-full items-center justify-center">
						<p class="text-xl text-base-content/50">{$_('new_window')}</p>
					</div>
				{:else}
					{#each currentChat.messages as message}
						{#if message.owner === 'User'}
							{@const textContent = message.content
								.filter((item) => 'Text' in item)
								.map((item) => item.Text)
								.join('')}
							{@const lineCount = (textContent.match(/\n/g) || []).length + 1}
							{@const shouldCollapse = textContent.length > 0 && lineCount > 10}
							<div class="chat-end chat">
								<div class="chat-bubble bg-primary/20 text-base-content">
									{#if shouldCollapse}
										{@const firstLine = textContent.split('\n')[0].slice(0, 80)}
										<details>
											<summary
												class="cursor-pointer text-xs font-medium text-base-content/70 hover:text-base-content"
											>
												<span class="font-normal">{firstLine}</span>
												<span class="ml-2 font-bold">
													... ({lineCount}{$_('folded_user_text')})
												</span>
											</summary>

											<div class="mt-2">
												{#each message.content as item}
													{#if 'Text' in item}
														<div class="prose max-w-none" on:click={handleMarkdownClick}>
															{@html renderMarkdown(item.Text)}
														</div>
													{:else if 'ImageRef' in item}
														<img
															src={getImageUrl(item)}
															alt={item.ImageRef[1]}
															class="my-2 h-auto w-64 cursor-pointer rounded-lg object-cover"
															on:click={() => showImageModal(getImageUrl(item))}
														/>
													{/if}
												{/each}
											</div>
										</details>
									{:else}
										{#each message.content as item}
											{#if 'Text' in item}
												<div class="prose max-w-none" on:click={handleMarkdownClick}>
													{@html renderMarkdown(item.Text)}
												</div>
											{:else if 'ImageRef' in item}
												<img
													src={getImageUrl(item)}
													alt={item.ImageRef[1]}
													class="my-2 h-auto w-64 cursor-pointer rounded-lg object-cover"
													on:click={() => showImageModal(getImageUrl(item))}
												/>
											{/if}
										{/each}
									{/if}
								</div>
							</div>
						{/if}

						{#if message.owner === 'Assistant'}
							<div class="chat-start chat">
								<div class="chat-bubble bg-base-200 text-base-content">
									{#if message.reasoning.length > 0}
										<details class="collapse-arrow collapse mb-2 bg-base-300/50 text-xs">
											<summary class="collapse-title min-h-0 py-2 font-medium">
												{$_('show_thinking')}
											</summary>
											<div class="collapse-content">
												{#each message.reasoning as item}
													{#if 'Text' in item}
														<div class="prose max-w-none">
															{@html renderMarkdown(item.Text, { disableImages: true })}
														</div>
													{/if}
												{/each}
											</div>
										</details>
									{/if}
									{#if (message.tool_deltas && message.tool_deltas.length > 0) || message.tool_use.length > 0}
										<details class="collapse-arrow collapse mt-2 bg-base-300/50 text-xs">
											<summary class="collapse-title min-h-0 py-2 font-medium">
												{#if message.tool_use.length > 0}
												{$_('tool_use')}
												{:else}
													<span class="flex items-center">
														<span class="loading mr-2 loading-xs loading-spinner"></span>
														{$_('generating_tools')}
													</span>
												{/if}
											</summary>

											<div class="collapse-content">
												{#if message.tool_use.length > 0}
													{#each message.tool_use as tool}
														<div class="my-1 rounded bg-base-100 p-2 font-mono">
															<p><strong>Tool:</strong> {tool.function_name}</p>
															<p><strong>Args:</strong></p>
															<pre class="whitespace-pre-wrap">{formatToolArgs(tool.args)}</pre>
														</div>
													{/each}
												{/if}
												{#if message.tool_deltas && message.tool_deltas.length > 0}
													<pre class="whitespace-pre-wrap">{message.tool_deltas}</pre>
												{/if}
											</div>
										</details>
									{/if}
									{#each message.content as item}
										{#if 'Text' in item}
											<div class="prose max-w-none" on:click={handleMarkdownClick}>
												{@html renderMarkdown(item.Text)}
											</div>
										{/if}
									{/each}
								</div>
							</div>
						{/if}
						{#if message.owner === 'Tools'}
							<details class="collapse-arrow collapse my-4 bg-base-300/50 text-sm">
								<summary class="collapse-title min-h-0 py-2 font-medium"
									>{$_('tool_result')}</summary
								>
								<div class="collapse-content">
									<div class="alert text-sm alert-info shadow-lg">
										<div class="w-full">
											<div class="my-2 grid grid-cols-3 gap-2">
												{#each message.content as item}
													{#if 'ImageRef' in item || 'ImageBin' in item}
														<img
															src={getImageUrl(item)}
															alt="Tool result"
															class="aspect-square w-full cursor-pointer rounded-lg object-cover"
															on:click={() => showImageModal(getImageUrl(item))}
														/>
													{/if}
												{/each}
											</div>

											{#each message.content as item}
												{#if 'Text' in item}
													<div class="prose max-w-none">
														{@html renderMarkdown(item.Text)}
													</div>
												{/if}
											{/each}
										</div>
									</div>
								</div>
							</details>
						{/if}
					{/each}
					<div id="chat-container-end"></div>
				{/if}
			</div>
			{#if showScrollToBottomButton}
				<button
					class="btn absolute right-8 z-10 btn-circle transition-all btn-primary"
					style="bottom: 10rem;"
					on:click={() => scrollToBottom(true)}
					title={$_('scroll_message')}
				>
					<svg
						xmlns="http://www.w3.org/2000/svg"
						fill="none"
						viewBox="0 0 24 24"
						stroke-width="2"
						stroke="currentColor"
						class="h-6 w-6"
					>
						<path
							stroke-linecap="round"
							stroke-linejoin="round"
							d="M19.5 13.5L12 21m0 0l-7.5-7.5M12 21V3"
						/>
					</svg>
				</button>
			{/if}
			<div class="relative border-t-2 border-base-300 bg-base-200 p-4">
				{#if previewFiles.length > 0}
					<div
						class="mb-2 flex max-h-40 flex-wrap gap-2 overflow-y-auto rounded-lg bg-base-100 p-2"
					>
						{#each previewFiles as item, i (item.url)}
							<div class="relative h-24 w-24 overflow-hidden rounded-lg">
								<img src={item.url} alt={item.file.name} class="h-full w-full object-cover" />
								<button
									class="btn absolute top-1 right-1 btn-circle btn-xs btn-error"
									on:click={() => removePreview(i)}
								>
									✕
								</button>
							</div>
						{/each}
					</div>
				{/if}

				<div class="flex items-start gap-2" role="group">
					<label class="btn btn-square shrink-0 btn-ghost">
						<svg
							xmlns="http://www.w3.org/2000/svg"
							fill="none"
							viewBox="0 0 24 24"
							stroke-width="1.5"
							stroke="currentColor"
							class="h-6 w-6"
						>
							<path
								stroke-linecap="round"
								stroke-linejoin="round"
								d="m2.25 15.75 5.159-5.159a2.25 2.25 0 0 1 3.182 0l5.159 5.159m-1.5-1.5 1.409-1.409a2.25 2.25 0 0 1 3.182 0l2.909 2.909m-18 3.75h16.5a1.5 1.5 0 0 0 1.5-1.5V6a1.5 1.5 0 0 0-1.5-1.5H3.75A1.5 1.5 0 0 0 2.25 6v12a1.5 1.5 0 0 0 1.5 1.5Zm10.5-11.25h.008v.008h-.008V8.25Zm.375 0a.375.375 0 1 1-.75 0 .375.375 0 0 1 .75 0Z"
							/>
						</svg>
						<input
							type="file"
							multiple
							accept="image/*"
							class="hidden"
							on:change={(e) => handleFileSelect(e.currentTarget.files)}
						/>
					</label>

					<textarea
						bind:value={textInput}
						class="textarea-bordered textarea flex-1"
						placeholder={$_('input_placeholder')}
						rows="1"
						disabled={!currentChat || processingChatIds.has(currentChat.id)}
						on:keydown={(e) => {
							if (e.key === 'Enter' && !e.shiftKey) {
								e.preventDefault();
								sendMessage();
							}
						}}
					></textarea>

					<button
						class="btn shrink-0 btn-primary"
						on:click={sendMessage}
						disabled={!currentChat ||
							processingChatIds.has(currentChat.id) ||
							(textInput.trim() === '' && previewFiles.length === 0)}
					>
						{#if currentChat && processingChatIds.has(currentChat.id)}
							<span class="loading loading-spinner"></span>
						{:else}
    						{$_('send')}
						{/if}
					</button>
				</div>
			</div>
			{#if isDragging}
				<div
					class="pointer-events-none absolute inset-0 z-10 flex items-center justify-center border-4 border-dashed border-primary bg-primary/20"
				>
					<span class="text-2xl font-bold text-primary">{$_('drag_zone')}</span>
				</div>
			{/if}
		</div>
	</div>
{/if}
