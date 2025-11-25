<script lang="ts">
	import { currentChat, isLoading, processingChatIds } from '$lib/stores/chatStore';
	import * as ChatService from '$lib/services/chatService';
	import { _ } from 'svelte-i18n';
	import MarkdownBlock from './MarkdownBlock.svelte'; // 假设你已分离
	import { createEventDispatcher, tick } from 'svelte';
	import type { MessageContent } from '$lib/types';
	import { toasts } from '$lib/stores/chatStore';
	import { settings } from '$lib/stores/settingsStore';
	import { playgroundState } from '$lib/stores/artifactStore';
	import { showArtifacts } from '$lib/stores/settingsStore';
	import CollapsibleText from './CollapsibleText.svelte';

	const dispatch = createEventDispatcher();
	let chatContainer: HTMLElement;
	let autoScrollEnabled = true;
	let showScrollToBottomButton = false;
	let editingMessageId: string | null = null;
	let editText = '';
	let hoveredToolId: string | null = null;
	let activeTabMap: Record<string, string> = {};

	function selectTab(groupId: string, msgId: string) {
		activeTabMap[groupId] = msgId;
		activeTabMap = { ...activeTabMap }; // 触发 Svelte 响应式更新
	}

	function getToolResultGroup(messages: any[], index: number) {
		const current = messages[index];
		const prev = messages[index - 1];

		if (current.owner.role !== 'tool') return null;
		if (prev && prev.owner.role === 'tool') return null;

		const group = [current];
		for (let i = index + 1; i < messages.length; i++) {
			if (messages[i].owner.role === 'tool') {
				group.push(messages[i]);
			} else {
				break;
			}
		}
		return group;
	}
	function stringToColor(str: string) {
		let hash = 0;
		for (let i = 0; i < str.length; i++) {
			hash = str.charCodeAt(i) + ((hash << 5) - hash);
		}
		// 生成 HSL 颜色：色相(0-360), 饱和度(60-80%), 亮度(85-95%) -> 柔和背景色
		const h = Math.abs(hash % 360);
		return `hsl(${h}, 75%, 90%)`;
	}

	function stringToBorderColor(str: string) {
		let hash = 0;
		for (let i = 0; i < str.length; i++) {
			hash = str.charCodeAt(i) + ((hash << 5) - hash);
		}
		// 边框颜色稍微深一点
		const h = Math.abs(hash % 360);
		return `hsl(${h}, 75%, 60%)`;
	}

	function getShortId(uuid: string) {
		return uuid.slice(0, 6);
	}

	function startEdit(message: any) {
		editingMessageId = message.id;
		// 提取纯文本用于编辑
		editText = message.content
			.filter((c: any) => 'Text' in c)
			.map((c: any) => c.Text)
			.join('');
	}

	function cancelEdit() {
		editingMessageId = null;
		editText = '';
	}

	async function submitEdit(id: string) {
		if (!editText.trim()) return;
		// 调用 Service
		await ChatService.editMessage(id, editText);
		editingMessageId = null;
	}

	function copyToPlayground(toolName: string, args: string) {
		$showArtifacts = true;
		playgroundState.set({ toolName, args });
	}

	// 处理 Textarea 的快捷键
	function handleEditKeydown(e: KeyboardEvent, id: string) {
		if ($settings.enterToSend) {
			if (e.key === 'Enter' && !e.shiftKey) {
				e.preventDefault();
				submitEdit(id);
			} else if (e.key === 'Escape') {
				cancelEdit();
			}
		} else {
			if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
				e.preventDefault();
				submitEdit(id);
			} else if (e.key === 'Escape') {
				cancelEdit();
			}
		}
	}
	// --- 工具函数 (用于模板渲染) ---
	function getImageUrl(item: MessageContent): string | null {
		if ('ImageRef' in item) return `/api/image/${item.ImageRef[0]}`;
		if ('ImageBin' in item) return `data:image/jpeg;base64,${item.ImageBin[0]}`;
		return null;
	}

	function formatToolArgs(args: string): string {
		try {
			return JSON.stringify(JSON.parse(args), null, 2);
		} catch {
			return args;
		}
	}

	// --- 滚动逻辑 ---
	export async function scrollToBottom(force = false) {
		if (!autoScrollEnabled && !force) return;
		if (force) {
			autoScrollEnabled = true;
			showScrollToBottomButton = false;
		}
		await tick();
		document.getElementById('chat-container-end')?.scrollIntoView({ behavior: 'smooth' });
	}

	function handleScroll() {
		if (!chatContainer) return;
		const threshold = 25;
		const isNearBottom =
			chatContainer.scrollHeight - chatContainer.scrollTop - chatContainer.clientHeight < threshold;

		autoScrollEnabled = isNearBottom;
		showScrollToBottomButton = !isNearBottom;
	}

	// 监听消息变化自动滚动
	let lastMsgCount = 0;
	let lastContentLength = 0;
	$: {
		if ($currentChat) {
			const msgs = $currentChat.messages;
			const lastMsg = msgs[msgs.length - 1];

			// 计算最后一条消息的正文长度 (忽略 reasoning)
			let currentContentLength = 0;
			if (lastMsg && lastMsg.owner.role === 'assistant') {
				currentContentLength = lastMsg.content.reduce(
					(acc, item) => acc + ('Text' in item ? item.Text.length : 0),
					0
				);
			}

			if (msgs.length !== lastMsgCount || currentContentLength !== lastContentLength) {
				lastMsgCount = msgs.length;
				lastContentLength = currentContentLength;

				// 稍微延时一点，等待 DOM 渲染
				setTimeout(() => scrollToBottom(autoScrollEnabled), 50);
			}
		}
	}

	function onImageClick(src: string) {
		dispatch('imageClick', src);
	}

	function handleContainerClick(e: MouseEvent) {
		const target = e.target as HTMLElement;
		const btn = target.closest('.copy-code-btn') as HTMLButtonElement;

		if (btn) {
			const container = btn.closest('.code-card');
			if (!container) return;

			const codeBlock = container.querySelector('code');

			if (codeBlock && codeBlock.textContent) {
				navigator.clipboard.writeText(codeBlock.textContent);

				const originalHTML = btn.innerHTML;
				btn.innerHTML = `<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#4ade80" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"/></svg>`;

				toasts.show('Copied!', 'success', 1000);

				setTimeout(() => {
					btn.innerHTML = originalHTML;
				}, 2000);
			}
			return;
		}

		if (target.tagName === 'IMG') {
			const src = target.getAttribute('src');
			if (src) {
				onImageClick(src);
			}
		}
	}

	function scrollToBottomAction(node: HTMLElement, text: string | undefined) {
		const scroll = () => {
			node.scrollTop = node.scrollHeight;
		};

		scroll();

		return {
			update(newText: string) {
				scroll();
			}
		};
	}

	function copyRawMessage(text: string) {
		navigator.clipboard.writeText(text);
		toasts.show('Raw markdown copied!', 'success', 1000);
	}
</script>

<div
	class="flex-1 overflow-y-auto bg-base-100 p-4 md:p-6"
	bind:this={chatContainer}
	on:scroll={handleScroll}
	on:click={handleContainerClick}
>
	{#if $isLoading && !$currentChat}
		<div class="flex h-full items-center justify-center">
			<span class="loading loading-lg loading-spinner text-primary"></span>
		</div>
	{:else if !$currentChat}
		<div class="flex h-full flex-col items-center justify-center opacity-50">
			<div class="mb-4 rounded-2xl bg-base-200 p-6">
				<svg
					xmlns="http://www.w3.org/2000/svg"
					fill="none"
					viewBox="0 0 24 24"
					stroke-width="1.5"
					stroke="currentColor"
					class="h-12 w-12"
				>
					<path
						stroke-linecap="round"
						stroke-linejoin="round"
						d="M8.625 12a.375.375 0 11-.75 0 .375.375 0 01.75 0zm0 0H8.25m4.125 0a.375.375 0 11-.75 0 .375.375 0 01.75 0zm0 0H12m4.125 0a.375.375 0 11-.75 0 .375.375 0 01.75 0zm0 0h-.375M21 12c0 4.556-4.03 8.25-9 8.25a9.764 9.764 0 01-2.555-.337A5.972 5.972 0 015.41 20.97a5.969 5.969 0 01-.474-.065 4.48 4.48 0 00.978-2.025c.09-.457-.133-.901-.467-1.226C3.93 16.178 3 14.159 3 12c0-4.556 4.03-8.25 9-8.25s9 3.694 9 8.25z"
					/>
				</svg>
			</div>
			<p class="text-xl font-medium">{$_('new_window')}</p>
		</div>
	{:else}
		<div class="mx-auto flex max-w-4xl flex-col gap-6">
			{#each $currentChat.messages as message, i (message.id)}
				{#if message.owner.role === 'user'}
					<div class="group chat-end chat">
						<div class="chat-header mb-1 opacity-0 transition-opacity group-hover:opacity-100">
							{#if editingMessageId !== message.id && !$processingChatIds.has($currentChat.id)}
								<button
									class="btn btn-circle text-base-content/50 btn-ghost btn-xs"
									on:click={() => startEdit(message)}
									title="Edit"
								>
									<svg
										xmlns="http://www.w3.org/2000/svg"
										viewBox="0 0 20 20"
										fill="currentColor"
										class="h-3 w-3"
									>
										<path
											d="M5.433 13.917l1.262-3.155A4 4 0 017.58 9.42l6.92-6.918a2.121 2.121 0 013 3l-6.92 6.918c-.383.383-.84.685-1.343.886l-3.154 1.262a.5.5 0 01-.65-.65z"
										/>
										<path
											d="M3.5 5.75c0-.69.56-1.25 1.25-1.25H10A.75.75 0 0010 3H4.75A2.75 2.75 0 002 5.75v9.5A2.75 2.75 0 004.75 18h9.5A2.75 2.75 0 0017 15.25V10a.75.75 0 00-1.5 0v5.25c0 .69-.56 1.25-1.25 1.25h-9.5c-.69 0-1.25-.56-1.25-1.25v-9.5z"
										/>
									</svg>
								</button>

								<button
									class="btn btn-circle text-base-content/50 btn-ghost btn-xs"
									on:click={() => ChatService.regenerateMessage(message.id)}
									title="Restart from here"
								>
									<svg
										xmlns="http://www.w3.org/2000/svg"
										viewBox="0 0 20 20"
										fill="currentColor"
										class="h-3 w-3"
									>
										<path
											fill-rule="evenodd"
											d="M15.312 11.424a5.5 5.5 0 01-9.201 2.466l-.312-.311h2.433a.75.75 0 000-1.5H3.989a.75.75 0 00-.75.75v4.242a.75.75 0 001.5 0v-2.43l.31.31a7 7 0 0011.712-3.138.75.75 0 00-1.449-.39zm1.23-3.723a.75.75 0 00.219-.53V2.929a.75.75 0 00-1.5 0V5.36l-.31-.31A7 7 0 003.239 8.188a.75.75 0 101.448.389A5.5 5.5 0 0113.89 6.11l.311.31h-2.432a.75.75 0 000 1.5h4.243a.75.75 0 00.53-.219z"
											clip-rule="evenodd"
										/>
									</svg>
								</button>
							{/if}
						</div>
						<div
							class="chat-bubble border border-primary/10 bg-primary/10 text-base-content shadow-sm"
						>
							{#if editingMessageId === message.id}
								<div class="flex flex-col gap-2 p-1">
									<textarea
										class="textarea-bordered textarea w-full bg-white/50"
										rows="3"
										bind:value={editText}
										on:keydown={(e) => handleEditKeydown(e, message.id)}
										autofocus
									></textarea>
									<div class="flex justify-end gap-2">
										<button class="btn btn-ghost btn-xs" on:click={cancelEdit}>Cancel</button>
										<button class="btn btn-xs btn-primary" on:click={() => submitEdit(message.id)}
											>Save & Submit</button
										>
									</div>
								</div>
							{:else}
								{#each message.content as item}
									{#if 'Text' in item}
										<CollapsibleText
											text={item.Text}
											on:imageClick={(e) => onImageClick(e.detail)}
										/>
									{:else if 'ImageRef' in item || 'ImageBin' in item}
										<div
											class="relative h-24 w-24 overflow-hidden rounded-lg border border-base-300 bg-base-100 shadow-sm transition-transform hover:scale-105"
										>
											<img
												src={getImageUrl(item)}
												alt="User Upload"
												class="h-full w-full cursor-zoom-in object-cover"
												on:click={() => {
													const url = getImageUrl(item);
													if (url) onImageClick(url);
												}}
											/>
										</div>
									{:else if 'AssetRef' in item}
										{@const [uuid, desc] = item.AssetRef}
										<a
											href={`/api/asset/${uuid}`}
											target="_blank"
											download={desc || `${uuid}` || 'download'}
											class="flex h-24 w-24 flex-col items-center justify-center gap-1 overflow-hidden rounded-lg border border-base-300 bg-base-200 p-2 shadow-sm transition-colors hover:bg-base-300"
											title={desc}
										>
											<svg
												xmlns="http://www.w3.org/2000/svg"
												viewBox="0 0 24 24"
												fill="currentColor"
												class="h-8 w-8 text-base-content/50"
											>
												<path
													fill-rule="evenodd"
													d="M5.625 1.5H9a.375.375 0 01.375.375v1.875c0 1.036.84 1.875 1.875 1.875H12.975c.966 0 1.755-.79 1.755-1.755V2.325c0-.427.453-.669.784-.42 2.633 1.977 3.732 3.58 3.732 8.095v9.75c0 2.071-1.679 3.75-3.75 3.75H9.75a3.75 3.75 0 01-3.75-3.75V2.25c0-.414.336-.75.75-.75zm6.75 8.25a.75.75 0 00-1.5 0v2.904l-.22-.22a.75.75 0 00-1.06 1.06l1.5 1.5a.75.75 0 001.06 0l1.5-1.5a.75.75 0 00-1.06-1.06l-.22.22V9.75z"
													clip-rule="evenodd"
												/>
											</svg>
											<span
												class="w-full truncate text-center text-[10px] leading-tight font-medium opacity-80"
											>
												{desc || 'File'}
											</span>
											<span class="badge scale-75 badge-xs font-mono badge-neutral">ASSET</span>
										</a>
									{/if}
								{/each}
							{/if}
						</div>
					</div>
				{/if}
				{#if message.owner.role === 'assistant'}
					<div class="group flex gap-4 pl-2">
						<div class="mt-1 flex-shrink-0">
							<div
								class="flex h-8 w-8 items-center justify-center rounded-full bg-gradient-to-br from-primary to-secondary text-white shadow-sm"
							>
								<svg
									xmlns="http://www.w3.org/2000/svg"
									fill="none"
									viewBox="0 0 24 24"
									stroke-width="2"
									stroke="currentColor"
									class="h-5 w-5"
								>
									<path
										stroke-linecap="round"
										stroke-linejoin="round"
										d="M9.813 15.904L9 18.75l-.813-2.846a4.5 4.5 0 00-3.09-3.09L2.25 12l2.846-.813a4.5 4.5 0 003.09-3.09L9 5.25l.813 2.846a4.5 4.5 0 003.09 3.09L15.75 12l-2.846.813a4.5 4.5 0 00-3.09 3.09zM18.259 8.715L18 9.75l-.259-1.035a3.375 3.375 0 00-2.455-2.456L14.25 6l1.036-.259a3.375 3.375 0 002.455-2.456L18 2.25l.259 1.035a3.375 3.375 0 002.456 2.456L21.75 6l-1.035.259a3.375 3.375 0 00-2.456 2.456zM16.894 20.567L16.5 21.75l-.394-1.183a2.25 2.25 0 00-1.423-1.423L13.5 18.75l1.183-.394a2.25 2.25 0 001.423-1.423l.394-1.183.394 1.183a2.25 2.25 0 001.423 1.423l1.183.394-1.183.394a2.25 2.25 0 00-1.423 1.423z"
									/>
								</svg>
							</div>
						</div>

						<div class="min-w-0 flex-1 space-y-2">
							{#if message.reasoning.length > 0}
								{@const reasoningText = message.reasoning
									.map((r) => ('Text' in r ? r.Text : ''))
									.join('')}
								<details
									class="group/think bg-base-50 collapse-arrow collapse rounded-lg border border-base-200"
									open
								>
									<summary
										class="collapse-title min-h-0 py-2 text-xs font-medium text-base-content/60 transition-colors hover:text-primary"
									>
										<div class="flex items-center gap-2">
											{$_('show_thinking')}
											{#if message.content.length === 0 && message.tool_use.length == 0}
												<span class="loading loading-xs loading-dots opacity-50"></span>
											{/if}
										</div>
									</summary>
									<div class="collapse-content">
										<div
											class="max-h-96 overflow-y-auto rounded-md bg-base-200/30 p-2 text-xs text-base-content/70"
											use:scrollToBottomAction={reasoningText}
										>
											{#each message.reasoning as item}
												{#if 'Text' in item}
													<div class="prose-xs prose max-w-none">
														<MarkdownBlock content={item.Text} disableImages />
													</div>
												{/if}
											{/each}
											<div class="h-2"></div>
										</div>
									</div>
								</details>
							{/if}

							{#if (message.tool_deltas && message.tool_deltas.length > 0) || message.tool_use.length > 0}
								<div class="relative z-10 my-2 flex flex-wrap gap-2">
									{#each message.tool_use as tool}
										{@const toolColor = stringToColor(tool.use_id)}
										{@const borderColor = stringToBorderColor(tool.use_id)}
										{@const isActive = hoveredToolId === tool.use_id}

										<div class="dropdown dropdown-start dropdown-bottom">
											<div
												tabindex="0"
												role="button"
												class="inline-flex cursor-pointer items-center gap-2 rounded-full border px-3 py-1.5 font-mono text-xs transition-all duration-200 hover:opacity-80"
												style="
																background-color: {toolColor};
																border-color: {borderColor};
																color: {borderColor};
																{isActive ? 'transform: scale(1.05); box-shadow: 0 2px 8px ' + toolColor : ''}
															"
												on:mouseenter={() => (hoveredToolId = tool.use_id)}
												on:mouseleave={() => (hoveredToolId = null)}
											>
												<svg
													xmlns="http://www.w3.org/2000/svg"
													viewBox="0 0 20 20"
													fill="currentColor"
													class="h-3 w-3 opacity-70"
												>
													<path
														fill-rule="evenodd"
														d="M19 5.5a4.5 4.5 0 01-4.791 4.483c-.353.048-.709.09-1.07.126V12.5c0 .414-.336.75-.75.75h-1.5a.75.75 0 01-.75-.75v-3.086a6.009 6.009 0 01-2.132-1.414l-.259.26a1.475 1.475 0 002.085 2.084l.515-.515a.75.75 0 111.06 1.06l-.515.515a2.975 2.975 0 01-4.206-4.206l.515-.515a.75.75 0 011.06 1.06l-.515.515a1.475 1.475 0 002.085 2.084l.26-.258a6.003 6.003 0 01-3.087-2.132H4.25a.75.75 0 01-.75-.75v-1.5c0-.414.336-.75.75-.75h3.086c.047-.36.089-.716.126-1.07A4.483 4.483 0 0112.5 1H14.5A4.5 4.5 0 0119 5.5z"
														clip-rule="evenodd"
													/>
												</svg>
												<span class="font-bold">{tool.function_name}</span>
												<span class="ml-1 rounded bg-white/50 px-1 text-[9px] opacity-70"
													>#{getShortId(tool.use_id)}</span
												>
											</div>

											<div
												tabindex="0"
												class="dropdown-content z-[100] mt-1 w-80 rounded-box border border-base-300 bg-base-100 p-2 shadow-xl sm:w-96"
											>
												<div class="flex flex-col gap-1 p-2">
													<div class="flex items-center justify-between">
														<span class="text-xs font-bold text-base-content/60">Arguments</span>
														<button
															class="btn text-primary btn-ghost btn-xs"
															on:click={() => copyToPlayground(tool.function_name, tool.args)}
															title="Edit in Playground"
														>
															<svg
																xmlns="http://www.w3.org/2000/svg"
																viewBox="0 0 20 20"
																fill="currentColor"
																class="mr-1 h-3 w-3"
																><path
																	d="M5.433 13.917l1.262-3.155A4 4 0 017.58 9.42l6.92-6.918a2.121 2.121 0 013 3l-6.92 6.918c-.383.383-.84.685-1.343.886l-3.154 1.262a.5.5 0 01-.65-.65z"
																/></svg
															>
															Edit
														</button>
													</div>
													<pre
														class="max-h-60 overflow-y-auto rounded bg-base-200/50 p-2 font-mono text-xs break-words whitespace-pre-wrap text-primary">{formatToolArgs(
															tool.args
														)}</pre>
												</div>
											</div>
										</div>
									{/each}

									{#if message.tool_deltas && message.tool_deltas.length > 0}
										<div class="group/badge relative inline-block">
											<div
												class="inline-flex animate-pulse items-center gap-2 rounded-full bg-base-200 px-3 py-1.5 text-xs text-base-content/60"
											>
												<span class="loading loading-xs loading-dots"></span>
												Calling tools...
											</div>
											<div
												class="pointer-events-none absolute top-full left-0 z-50 mt-2 w-64 opacity-0 transition-all duration-200 group-hover:pointer-events-auto group-hover:opacity-100 sm:w-80"
											>
												<div
													class="rounded-lg border border-base-300 bg-base-100 p-3 text-xs shadow-xl"
												>
													<div
														class="flex items-center justify-between font-bold text-base-content/60"
													>
														<span>Live Output</span><span
															class="h-2 w-2 animate-pulse rounded-full bg-success"
														></span>
													</div>
													<pre
														class="max-h-40 overflow-y-auto rounded bg-base-300 p-2 font-mono text-[10px] break-all whitespace-pre-wrap"
														use:scrollToBottomAction={message.tool_deltas}>{message.tool_deltas}</pre>
												</div>
											</div>
										</div>
									{:else if message.tool_use.length === 0}
										<span class="loading loading-xs loading-dots opacity-50"></span>
									{/if}
								</div>
							{/if}
							<div class="prose prose-sm max-w-none text-base-content">
								{#each message.content as item}
									{#if 'Text' in item}
										<MarkdownBlock
											content={item.Text}
											on:imageClick={(e) => onImageClick(e.detail)}
										/>
									{/if}
								{/each}
							</div>

							<div
								class="flex items-center gap-2 pt-2 opacity-0 transition-opacity group-hover:opacity-100"
							>
								<button
									class="btn gap-1 text-base-content/50 btn-ghost btn-xs"
									on:click={() => {
										const raw = message.content
											.filter((c) => 'Text' in c)
											.map((c) => c.Text)
											.join('\n');
										copyRawMessage(raw);
									}}
								>
									<svg
										xmlns="http://www.w3.org/2000/svg"
										viewBox="0 0 20 20"
										fill="currentColor"
										class="h-3 w-3"
										><path
											d="M7 3.5A1.5 1.5 0 018.5 2h3.879a1.5 1.5 0 011.06.44l3.122 3.12A1.5 1.5 0 0117 6.622V12.5a1.5 1.5 0 01-1.5 1.5h-1v-3.379a3 3 0 00-.879-2.121L10.5 5.379A3 3 0 008.379 4.5H7v-1z"
										/><path
											d="M4.5 6A1.5 1.5 0 003 7.5v9A1.5 1.5 0 004.5 18h7.5a1.5 1.5 0 001.5-1.5v-5.879a.75.75 0 00-.22-.53L9.78 6.53A.75.75 0 009.25 6H4.5z"
										/></svg
									>
									Copy Raw
								</button>
								<button
									class="btn gap-1 text-base-content/50 btn-ghost btn-xs hover:text-primary"
									disabled={$processingChatIds.has($currentChat?.id || '')}
									on:click={() => ChatService.regenerateMessage(message.id)}
									title="Regenerate response"
								>
									<svg
										xmlns="http://www.w3.org/2000/svg"
										viewBox="0 0 20 20"
										fill="currentColor"
										class="h-3 w-3"
										><path
											fill-rule="evenodd"
											d="M15.312 11.424a5.5 5.5 0 01-9.201 2.466l-.312-.311h2.433a.75.75 0 000-1.5H3.989a.75.75 0 00-.75.75v4.242a.75.75 0 001.5 0v-2.43l.31.31a7 7 0 0011.712-3.138.75.75 0 00-1.449-.39zm1.23-3.723a.75.75 0 00.219-.53V2.929a.75.75 0 00-1.5 0V5.36l-.31-.31A7 7 0 003.239 8.188a.75.75 0 101.448.389A5.5 5.5 0 0113.89 6.11l.311.31h-2.432a.75.75 0 000 1.5h4.243a.75.75 0 00.53-.219z"
											clip-rule="evenodd"
										/></svg
									>
									Regenerate
								</button>
							</div>
						</div>
					</div>
				{/if}
				{#if message.owner.role === 'tool'}
					{@const toolGroup = getToolResultGroup($currentChat.messages, i)}

					{#if toolGroup}
						{@const groupId = message.id}
						{@const activeMsgId = activeTabMap[groupId] || toolGroup[0].id}
						{@const activeMsg = toolGroup.find((m) => m.id === activeMsgId) || toolGroup[0]}

						<details
							class="group collapse-arrow collapse mb-2 ml-12 rounded-lg border border-base-200 bg-base-100 shadow-sm"
							open={true}
						>
							<summary
								class="collapse-title min-h-0 py-2 pr-4 text-xs font-medium text-base-content/60 transition-colors hover:bg-base-200/50 hover:text-primary"
							>
								<div class="flex items-center gap-2">
									<svg
										xmlns="http://www.w3.org/2000/svg"
										viewBox="0 0 20 20"
										fill="currentColor"
										class="h-4 w-4"
									>
										<path
											fill-rule="evenodd"
											d="M2 10a.75.75 0 01.75-.75h12.59l-2.1-1.95a.75.75 0 111.02-1.1l3.5 3.25a.75.75 0 010 1.1l-3.5 3.25a.75.75 0 11-1.02-1.1l2.1-1.95H2.75A.75.75 0 012 10z"
											clip-rule="evenodd"
										/>
									</svg>
									<span class="tracking-wider uppercase">Tool Outputs</span>
									<span class="badge badge-ghost badge-sm font-mono text-[10px]"
										>{toolGroup.length} results</span
									>
								</div>
							</summary>

							<div class="collapse-content px-0 pb-0">
								<div class="flex flex-col">
									<div
										class="bg-base-50/50 scrollbar-hide flex items-center gap-2 overflow-x-auto border-b border-base-200 px-2 py-1.5"
									>
										{#each toolGroup as msg}
											{@const toolCallId = msg.owner.tool_call_id}
											{@const borderColor = stringToBorderColor(toolCallId)}
											{@const isActive = activeMsg.id === msg.id}

											<button
												class="relative flex flex-shrink-0 items-center gap-1.5 rounded-md border px-2 py-1 text-[10px] font-medium transition-all hover:bg-base-200"
												style="
                                    border-color: {borderColor};
                                    {isActive
													? `background-color: ${borderColor}15; color: ${borderColor};`
													: 'border-color: transparent; opacity: 0.6;'}
                                "
												on:click={() => selectTab(groupId, msg.id)}
												on:mouseenter={() => (hoveredToolId = toolCallId)}
												on:mouseleave={() => (hoveredToolId = null)}
											>
												<div
													class="h-1.5 w-1.5 rounded-full"
													style="background-color: {borderColor}"
												></div>
												<span class="font-mono">#{getShortId(toolCallId)}</span>
											</button>
										{/each}
									</div>

									<div class="bg-base-100 p-3">
										{#key activeMsg.id}
											<div class="animate-in fade-in flex flex-col gap-2 duration-200">
												{#if activeMsg.content.some((item) => 'ImageRef' in item || 'ImageBin' in item)}
													<div class="grid grid-cols-3 gap-2">
														{#each activeMsg.content as item}
															{#if 'ImageRef' in item || 'ImageBin' in item}
																<div
																	class="bg-base-50 aspect-square overflow-hidden rounded border border-base-200"
																>
																	<img
																		src={getImageUrl(item)}
																		alt="Result"
																		class="h-full w-full cursor-pointer object-cover transition-transform hover:scale-105"
																		on:click={() => {
																			const url = getImageUrl(item);
																			if (url) onImageClick(url);
																		}}
																	/>
																</div>
															{/if}
														{/each}
													</div>
												{/if}

												{#each activeMsg.content as item}
													{#if 'Text' in item}
														<div
															class="relative overflow-hidden rounded bg-base-200/30 p-2 font-mono text-xs text-base-content/80"
														>
															<div
																class="scrollbar-thin max-h-60 overflow-y-auto break-words whitespace-pre-wrap"
															>
																{item.Text}
															</div>
														</div>
													{/if}
												{/each}
											</div>
										{/key}
									</div>
								</div>
							</div>
						</details>
					{/if}
				{/if}
			{/each}
			<div id="chat-container-end" class="h-12"></div>
		</div>
	{/if}
</div>
