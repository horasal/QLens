<script lang="ts">
	import { currentChat, processingChatIds } from '$lib/stores/chatStore';
	import * as ChatService from '$lib/services/chatService';
	import { _ } from 'svelte-i18n';
	import type { PreviewFile } from '$lib/types';

	let textInput = '';
	let previewFiles: PreviewFile[] = [];

	// 对外暴露方法：允许父组件（拖拽逻辑）添加文件
	export function addFiles(files: FileList | null) {
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

	async function handleSend() {
		if (!$currentChat) {
			await ChatService.startNewChat();
		}
		if (textInput.trim() === '' && previewFiles.length === 0) return;

		await ChatService.sendMessage(textInput, previewFiles);

		// 发送成功后清空
		textInput = '';
		previewFiles = [];
	}

	function handleStop() {
		if ($currentChat) {
			ChatService.abortGeneration($currentChat.id);
		}
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Enter' && !e.shiftKey) {
			e.preventDefault();
			handleSend();
		}
	}

	$: isProcessing = $currentChat && $processingChatIds.has($currentChat.id);
</script>

<div class="relative border-t-2 border-base-300 bg-base-200 p-4">
	{#if previewFiles.length > 0}
		<div class="mb-2 flex max-h-40 flex-wrap gap-2 overflow-y-auto rounded-lg bg-base-100 p-2">
			{#each previewFiles as item, i (item.url)}
				<div class="relative h-24 w-24 overflow-hidden rounded-lg">
					<img src={item.url} alt={item.file.name} class="h-full w-full object-cover" />
					<button
						class="btn absolute right-1 top-1 btn-circle btn-error btn-xs"
						on:click={() => removePreview(i)}
					>
						✕
					</button>
				</div>
			{/each}
		</div>
	{/if}

	<div class="flex items-start gap-2">
		<label class="btn btn-square btn-ghost shrink-0">
			<svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" class="h-6 w-6">
				<path stroke-linecap="round" stroke-linejoin="round" d="m2.25 15.75 5.159-5.159a2.25 2.25 0 0 1 3.182 0l5.159 5.159m-1.5-1.5 1.409-1.409a2.25 2.25 0 0 1 3.182 0l2.909 2.909m-18 3.75h16.5a1.5 1.5 0 0 0 1.5-1.5V6a1.5 1.5 0 0 0-1.5-1.5H3.75A1.5 1.5 0 0 0 2.25 6v12a1.5 1.5 0 0 0 1.5 1.5Zm10.5-11.25h.008v.008h-.008V8.25Zm.375 0a.375.375 0 1 1-.75 0 .375.375 0 0 1 .75 0Z" />
			</svg>
			<input type="file" multiple accept="image/*" class="hidden" on:change={(e) => addFiles(e.currentTarget.files)} />
		</label>

		<textarea
			bind:value={textInput}
			class="textarea textarea-bordered flex-1"
			placeholder={$_('input_placeholder')}
			rows="1"
			disabled={!!isProcessing}
			on:keydown={handleKeydown}
		></textarea>

		<button
			class="btn min-w-[5rem] shrink-0 transition-all duration-200"
			class:btn-error={isProcessing}
			class:btn-primary={!isProcessing}
			on:click={isProcessing ? handleStop : handleSend}
			disabled={!isProcessing && textInput.trim() === '' && previewFiles.length === 0}
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
