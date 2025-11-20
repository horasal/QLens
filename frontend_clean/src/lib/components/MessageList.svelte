<script lang="ts">
	import { currentChat, isLoading, processingChatIds } from '$lib/stores/chatStore';
	import * as ChatService from '$lib/services/chatService';
	import { _ } from 'svelte-i18n';
	import MarkdownBlock from './MarkdownBlock.svelte'; // 假设你已分离
	import { createEventDispatcher, tick, afterUpdate } from 'svelte';
	import type { MessageContent } from '$lib/types';

	const dispatch = createEventDispatcher();
	let chatContainer: HTMLElement;
	let autoScrollEnabled = true;
	let showScrollToBottomButton = false;

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
			if (lastMsg && lastMsg.owner === 'Assistant') {
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

		// 1. 处理代码块复制
		if (target.classList.contains('copy-code-btn')) {
			// 找到同级的 pre > code 元素
			const container = target.closest('.relative');
			const codeBlock = container?.querySelector('code');
			if (codeBlock && codeBlock.textContent) {
				navigator.clipboard.writeText(codeBlock.textContent);
				toasts.show('Code copied!', 'success', 1000);
			}
			return;
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
			{#each $currentChat.messages as message}
				{#if message.owner === 'User'}
					<div class="group chat-end chat">
						<div class="chat-header mb-1 opacity-0 transition-opacity group-hover:opacity-100">
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
						</div>
						<div
							class="chat-bubble border border-primary/10 bg-primary/10 text-base-content shadow-sm"
						>
							{#each message.content as item}
								{#if 'Text' in item}
									<div class="whitespace-pre-wrap">
										<MarkdownBlock
											content={item.Text}
											on:imageClick={(e) => onImageClick(e.detail)}
										/>
									</div>
								{:else}
									<img
										src={getImageUrl(item)}
										alt="User upload"
										class="my-2 h-auto w-64 cursor-pointer rounded-lg border-2 border-white/20 object-cover"
										on:click={() => {
											const url = getImageUrl(item);
											if (url) onImageClick(url);
										}}
									/>
								{/if}
							{/each}
						</div>
					</div>
				{/if}

				{#if message.owner === 'Assistant'}
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
									open={message.content.length === 0}
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
								<div class="my-2">
									{#if message.tool_use.length > 0}
										{#each message.tool_use as tool}
											<div class="dropdown dropdown-start dropdown-bottom">
												<div
													tabindex="0"
													role="button"
													class="inline-flex cursor-pointer items-center gap-2 rounded-full border border-base-300 bg-base-200 px-3 py-1.5 font-mono text-xs text-base-content/80 transition-colors hover:border-secondary hover:text-secondary"
												>
													<svg
														xmlns="http://www.w3.org/2000/svg"
														viewBox="0 0 20 20"
														fill="currentColor"
														class="h-3 w-3"
													>
														<path
															fill-rule="evenodd"
															d="M19 5.5a4.5 4.5 0 01-4.791 4.483c-.353.048-.709.09-1.07.126V12.5c0 .414-.336.75-.75.75h-1.5a.75.75 0 01-.75-.75v-3.086a6.009 6.009 0 01-2.132-1.414l-.259.26a1.475 1.475 0 002.085 2.084l.515-.515a.75.75 0 111.06 1.06l-.515.515a2.975 2.975 0 01-4.206-4.206l.515-.515a.75.75 0 011.06 1.06l-.515.515a1.475 1.475 0 002.085 2.084l.26-.258a6.003 6.003 0 01-3.087-2.132H4.25a.75.75 0 01-.75-.75v-1.5c0-.414.336-.75.75-.75h3.086c.047-.36.089-.716.126-1.07A4.483 4.483 0 0112.5 1H14.5A4.5 4.5 0 0119 5.5z"
															clip-rule="evenodd"
														/>
													</svg>
													<span class="font-bold">{tool.function_name}</span>
												</div>
												<div
													tabindex="0"
													class="dropdown-content z-[100] mt-1 w-80 rounded-box border border-base-300 bg-base-100 p-2 shadow-xl sm:w-96"
												>
													<div class="flex flex-col gap-1 p-2">
														<div class="text-xs font-bold text-base-content/60">Arguments</div>
														<pre
															class="max-h-60 overflow-y-auto rounded bg-base-200/50 p-2 font-mono text-xs break-words whitespace-pre-wrap text-primary">{formatToolArgs(
																tool.args
															)}</pre>
													</div>
												</div>
											</div>
										{/each}
									{:else}
										<div class="group relative inline-block">
											<div
												class="inline-flex animate-pulse items-center gap-2 rounded-full bg-base-200 px-3 py-1.5 text-xs text-base-content/60"
											>
												<span class="loading loading-xs loading-dots"></span>
												Calling tools...
											</div>
											{#if message.tool_deltas && message.tool_deltas.length > 0}
												<div
													class="pointer-events-none absolute top-full left-0 z-50 mt-2 w-64 opacity-0 transition-all duration-200 group-hover:pointer-events-auto group-hover:opacity-100 sm:w-80"
												>
													<div
														class="rounded-lg border border-base-300 bg-base-100 p-3 text-xs shadow-xl"
													>
														<div class="mb-1 font-bold text-base-content/60">Live Output</div>

														<div
															class="max-h-40 overflow-y-auto font-mono break-all whitespace-pre-wrap opacity-70"
															style="scrollbar-width: none; -ms-overflow-style: none;"
															use:scrollToBottomAction={message.tool_deltas}
														>
															<style>
																div::-webkit-scrollbar {
																	display: none;
																}
															</style>
															{message.tool_deltas}
														</div>
													</div>
												</div>
											{/if}
										</div>
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
										// 拼接所有 Text 内容
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
									>
										<path
											d="M7 3.5A1.5 1.5 0 018.5 2h3.879a1.5 1.5 0 011.06.44l3.122 3.12A1.5 1.5 0 0117 6.622V12.5a1.5 1.5 0 01-1.5 1.5h-1v-3.379a3 3 0 00-.879-2.121L10.5 5.379A3 3 0 008.379 4.5H7v-1z"
										/>
										<path
											d="M4.5 6A1.5 1.5 0 003 7.5v9A1.5 1.5 0 004.5 18h7.5a1.5 1.5 0 001.5-1.5v-5.879a.75.75 0 00-.22-.53L9.78 6.53A.75.75 0 009.25 6H4.5z"
										/>
									</svg>
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
									>
										<path
											fill-rule="evenodd"
											d="M15.312 11.424a5.5 5.5 0 01-9.201 2.466l-.312-.311h2.433a.75.75 0 000-1.5H3.989a.75.75 0 00-.75.75v4.242a.75.75 0 001.5 0v-2.43l.31.31a7 7 0 0011.712-3.138.75.75 0 00-1.449-.39zm1.23-3.723a.75.75 0 00.219-.53V2.929a.75.75 0 00-1.5 0V5.36l-.31-.31A7 7 0 003.239 8.188a.75.75 0 101.448.389A5.5 5.5 0 0113.89 6.11l.311.31h-2.432a.75.75 0 000 1.5h4.243a.75.75 0 00.53-.219z"
											clip-rule="evenodd"
										/>
									</svg>
									Regenerate
								</button>
							</div>
						</div>
					</div>
				{/if}

				{#if message.owner === 'Tools'}
					<details
						class="group collapse-arrow collapse mb-4 ml-10 rounded-r-lg border-l-4 border-secondary bg-base-200/30 shadow-sm"
					>
						<summary
							class="collapse-title min-h-0 py-2 text-xs font-bold tracking-wider text-secondary/80 uppercase transition-colors hover:bg-base-200/50"
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
								<span>{$_('tool_result')}</span>
							</div>
						</summary>

						<div class="collapse-content text-sm">
							<div class="flex flex-col gap-3 pt-2">
								{#if message.content.some((item) => 'ImageRef' in item || 'ImageBin' in item)}
									<div class="flex flex-wrap gap-2">
										{#each message.content as item}
											{#if 'ImageRef' in item || 'ImageBin' in item}
												<div
													class="relative overflow-hidden rounded-md border border-base-300 bg-base-100 transition-all hover:scale-105 hover:shadow-md"
												>
													<img
														src={getImageUrl(item)}
														alt="Tool result"
														class="h-32 w-32 cursor-pointer object-cover"
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

								{#each message.content as item}
									{#if 'Text' in item}
										<div
											class="relative overflow-hidden rounded bg-base-300/50 p-3 font-mono text-xs text-base-content/80"
										>
											<div class="max-h-60 overflow-y-auto break-words whitespace-pre-wrap">
												{item.Text}
											</div>
										</div>
									{/if}
								{/each}
							</div>
						</div>
					</details>
				{/if}
			{/each}
			<div id="chat-container-end" class="h-12"></div>
		</div>
	{/if}
</div>
