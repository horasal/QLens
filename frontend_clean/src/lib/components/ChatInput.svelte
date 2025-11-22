<script lang="ts">
	import { currentChat, processingChatIds, currentUsage, toasts } from '$lib/stores/chatStore';
	import * as ChatService from '$lib/services/chatService';
	import { _ } from 'svelte-i18n';
	import { settings } from '$lib/stores/settingsStore';
	import type { PendingFile } from '$lib/types';

	let textInput = '';
	let pendingFiles: PendingFile[] = [];

	const TEXT_SIZE_LIMIT = 2 * 1024;

	// 定义常见的文本/代码后缀
	const TEXT_EXTENSIONS = new Set([
		'txt', 'md', 'csv', 'json', 'yaml', 'toml', 'xml', 'html', 'css',
		'js', 'ts', 'rs', 'py', 'go', 'c', 'cpp', 'h', 'java', 'svelte',
		'vue', 'sh', 'bat', 'log', 'sql', 'ini', 'conf'
	]);

	function isTextFile(file: File): boolean {
		if (
			file.type.startsWith('text/') ||
			file.type === 'application/json' ||
			file.type.includes('javascript') ||
			file.type.includes('xml')
		)
			return true;
		const ext = file.name.split('.').pop()?.toLowerCase();
		return ext ? TEXT_EXTENSIONS.has(ext) : false;
	}

	async function handleFiles(fileList: FileList | null) {
		if (!fileList) return;
		const files = Array.from(fileList);

		for (const file of files) {
			const id = self.crypto.randomUUID();

			if (file.type.startsWith('image/')) {
				const reader = new FileReader();
				reader.onload = (e) => {
					const url = e.target?.result as string;
					pendingFiles = [...pendingFiles, { type: 'image', file, url, id }];
				};
				reader.readAsDataURL(file);
			}
			// 小文本文件处理 (< 2KB) -> MessageContent::Text
			else if (isTextFile(file) && file.size < TEXT_SIZE_LIMIT) {
				try {
					const text = await file.text();
					pendingFiles = [...pendingFiles, { type: 'text_content', file, content: text, id }];
					toasts.show(`Loaded ${file.name} as text snippet`, 'success');
				} catch (e) {
					toasts.show(`Failed to read ${file.name}`, 'error');
				}
			}
			// 其他文件或大文本 -> MessageContent::AssetRef
			else {
				pendingFiles = [...pendingFiles, { type: 'asset', file, id }];
				toasts.show(`Loaded ${file.name} as attachment`, 'info');
			}
		}
	}

	// 粘贴事件处理
	function handlePaste(e: ClipboardEvent) {
		if (e.clipboardData?.files && e.clipboardData.files.length > 0) {
			e.preventDefault();
			handleFiles(e.clipboardData.files);
		}
	}

	export function addFiles(files: FileList | null) {
		handleFiles(files);
	}

	function removeFile(index: number) {
		pendingFiles = pendingFiles.filter((_, i) => i !== index);
	}

	async function handleSend() {
		if (!$currentChat) {
			await ChatService.startNewChat();
		}
		if (textInput.trim() === '' && pendingFiles.length === 0) return;

		await ChatService.sendMessage(textInput, pendingFiles);

		textInput = '';
		pendingFiles = [];
	}

	function handleStop() {
		if ($currentChat) {
			ChatService.abortGeneration($currentChat.id);
		}
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.isComposing) return;
		if ($settings.enterToSend) {
			if (e.key === 'Enter' && !e.shiftKey) {
				e.preventDefault();
				handleSend();
			}
		} else {
			if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
				e.preventDefault();
				handleSend();
			}
		}
	}

	$: isProcessing = $currentChat && $processingChatIds.has($currentChat.id);
</script>

<div class="relative border-t-2 border-base-300 bg-base-200 p-4">
	{#if $currentUsage}
		<div class="absolute -top-8 right-2 flex gap-2 rounded-t-lg border-x border-t border-base-300 bg-base-100/50 px-2 py-1 font-mono text-xs text-base-content/60 backdrop-blur-sm">
			<span title="Prompt Tokens">In: {$currentUsage.prompt_tokens}</span>
			<span>|</span>
			<span title="Completion Tokens">Out: {$currentUsage.completion_tokens}</span>
			<span>|</span>
			<span class="font-bold" title="Total">Total: {$currentUsage.total_tokens}</span>
		</div>
	{/if}

	{#if pendingFiles.length > 0}
		<div class="mb-2 flex max-h-40 flex-wrap gap-2 overflow-y-auto rounded-lg bg-base-100 p-2">
			{#each pendingFiles as item, i (item.id)}
				<div class="relative group flex h-24 w-24 flex-col items-center justify-center overflow-hidden rounded-lg border border-base-300 bg-base-200 shadow-sm transition-all hover:shadow-md">

					<button
						class="btn btn-circle btn-error btn-xs absolute right-1 top-1 z-10 opacity-0 transition-opacity group-hover:opacity-100"
						on:click={() => removeFile(i)}
					>
						✕
					</button>

					{#if item.type === 'image'}
						<img src={item.url} alt={item.file.name} class="h-full w-full object-cover" />
					{:else}
						<div class="flex h-full w-full flex-col items-center justify-center gap-1 p-2">
							<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor" class="h-8 w-8 text-base-content/50">
								<path fill-rule="evenodd" d="M5.625 1.5H9a.375.375 0 01.375.375v1.875c0 1.036.84 1.875 1.875 1.875H12.975c.966 0 1.755-.79 1.755-1.755V2.325c0-.427.453-.669.784-.42 2.633 1.977 3.732 3.58 3.732 8.095v9.75c0 2.071-1.679 3.75-3.75 3.75H9.75a3.75 3.75 0 01-3.75-3.75V2.25c0-.414.336-.75.75-.75zm6.75 8.25a.75.75 0 00-1.5 0v2.904l-.22-.22a.75.75 0 00-1.06 1.06l1.5 1.5a.75.75 0 001.06 0l1.5-1.5a.75.75 0 00-1.06-1.06l-.22.22V9.75z" clip-rule="evenodd" />
							</svg>
							<span class="w-full truncate text-center text-[10px] font-medium leading-tight text-base-content/80" title={item.file.name}>
								{item.file.name}
							</span>

							{#if item.type === 'text_content'}
								<span class="badge badge-neutral badge-xs scale-75 font-mono">TXT</span>
							{:else}
								<span class="badge badge-primary badge-xs scale-75 font-mono">FILE</span>
							{/if}
						</div>
					{/if}
				</div>
			{/each}
		</div>
	{/if}

	<div class="relative border-t-2 border-base-300 bg-base-200 p-4">
		<div class="flex items-start gap-2">
			<label class="btn btn-square btn-ghost shrink-0" class:btn-disabled={!!isProcessing}>
				<svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" class="h-6 w-6">
					<path stroke-linecap="round" stroke-linejoin="round" d="m18.375 12.739-7.693 7.693a4.5 4.5 0 0 1-6.364-6.364l10.94-10.94A3 3 0 1 1 19.5 7.372L8.552 18.32m.009-.01-.01.01m5.699-9.941-7.81 7.81a1.5 1.5 0 0 0 2.112 2.13" />
				</svg>
				<input
					type="file"
					multiple
					class="hidden"
					disabled={!!isProcessing}
					on:change={(e) => addFiles(e.currentTarget.files)}
				/>
			</label>

			<textarea
				bind:value={textInput}
				class="textarea textarea-bordered flex-1"
				placeholder={$_('input_placeholder')}
				rows="1"
				disabled={!!isProcessing}
				on:keydown={handleKeydown}
				on:paste={handlePaste}
			></textarea>

			<button
				class="btn min-w-[5rem] shrink-0 transition-all duration-200"
				class:btn-error={isProcessing}
				class:btn-primary={!isProcessing}
				on:click={isProcessing ? handleStop : handleSend}
				disabled={!isProcessing && textInput.trim() === '' && pendingFiles.length === 0}
			>
				{#if isProcessing}
					<div class="flex items-center gap-2">
						<div class="relative h-5 w-5">
							<span class="loading loading-spinner loading-sm absolute inset-0 transition-opacity duration-200 group-hover:opacity-0"></span>
							<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor" class="absolute inset-0 h-5 w-5 opacity-0 transition-opacity duration-200 group-hover:opacity-100">
								<path fill-rule="evenodd" d="M4.5 7.5a3 3 0 013-3h9a3 3 0 013 3v9a3 3 0 01-3 3h-9a3 3 0 01-3-3v-9z" clip-rule="evenodd" />
							</svg>
						</div>
						<span>{$_('stop')}</span>
					</div>
				{:else}
					<div class="flex items-center gap-2">
						{$_('send')}
						<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor" class="h-5 w-5">
							<path d="M3.105 2.289a.75.75 0 00-.826.95l1.414 4.925A1.5 1.5 0 005.135 9.25h6.115a.75.75 0 010 1.5H5.135a1.5 1.5 0 00-1.442 1.086l-1.414 4.926a.75.75 0 00.826.95 28.896 28.896 0 0015.293-7.154.75.75 0 000-1.115A28.897 28.897 0 003.105 2.289z" />
						</svg>
					</div>
				{/if}
			</button>
		</div>
	</div>
</div>
