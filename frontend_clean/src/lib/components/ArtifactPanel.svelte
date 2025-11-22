<script lang="ts">
	import { currentChat } from '$lib/stores/chatStore';
	import * as ChatService from '$lib/services/chatService';
	import { onMount } from 'svelte';
	import type { ToolDescription, MessageContent, Message } from '$lib/types';
	import MarkdownBlock from './MarkdownBlock.svelte';
	import { playgroundState } from '$lib/stores/artifactStore';
	import { createEventDispatcher } from 'svelte';
	import { toasts } from '$lib/stores/chatStore';
	import { themeStore } from '$lib/stores/themeStore';
	import CodeMirror from 'svelte-codemirror-editor';
	import { javascript } from '@codemirror/lang-javascript';
	import { oneDark } from '@codemirror/theme-one-dark';
	import SchemaForm from './SchemaForm.svelte';

	const dispatch = createEventDispatcher();
	let formValue: Record<string, any> = {};
	let toolArgsCache: Record<string, string> = {};
	let activeTab: 'gallery' | 'playground' = 'gallery';

	// Playground State
	let tools: ToolDescription[] = [];
	let selectedToolName = 'js_interpreter';
	let inputArgs = '';
	let isRunning = false;
	let runResult: Message | null = null;

	type GalleryItem = {
		src: string;
		alt: string;
		code?: string; // 关联的生成代码/参数
		toolName?: string; // 关联的工具名
	};

	let galleryImages: GalleryItem[] = [];

	function copyUUID(e: MouseEvent, src: string) {
		const uuid = src.split('/').pop();
		if (uuid) {
			navigator.clipboard.writeText(uuid);
			toasts.show('UUID Copied!', 'success', 1000);
		}
	}

	function handleToolSwitch(e: Event) {
		const newToolName = (e.target as HTMLSelectElement).value;

		if (selectedToolName) {
			toolArgsCache[selectedToolName] = inputArgs;
		}

		selectedToolName = newToolName;

		if (toolArgsCache[newToolName] !== undefined) {
			inputArgs = toolArgsCache[newToolName];
		} else {
			inputArgs = newToolName === 'js_interpreter' ? '' : '{}';
		}

		if (selectedToolName !== 'js_interpreter') {
			try {
				formValue = JSON.parse(inputArgs || '{}');
			} catch {
				formValue = {};
			}
		}
	}

	function loadIntoPlayground(item: GalleryItem) {
		activeTab = 'playground';

		if (selectedToolName && item.toolName && selectedToolName !== item.toolName) {
			toolArgsCache[selectedToolName] = inputArgs;
		}

		if (item.toolName) selectedToolName = item.toolName;

		let codeToLoad = item.code || '';
		try {
			const parsed = JSON.parse(codeToLoad);
			codeToLoad = typeof parsed === 'string' ? parsed : JSON.stringify(parsed, null, 2);
		} catch {}

		inputArgs = codeToLoad;

		toolArgsCache[selectedToolName] = inputArgs;

		if (selectedToolName !== 'js_interpreter') {
			try {
				formValue = JSON.parse(inputArgs);
			} catch {
				formValue = {};
			}
		}
	}

	// 获取当前选中的 Tool 对象
	$: currentToolDef = tools.find((t) => t.name_for_model === selectedToolName);

	$: if ($currentChat) {
		const images: GalleryItem[] = [];
		const msgs = $currentChat.messages;

		msgs.forEach((msg, i) => {
			if (msg.owner !== 'Tools') return;

			// 尝试寻找上下文：通常前一条消息(Assistant)包含了 Tool Call
			// 注意：这是简化逻辑，严谨做法应该匹配 tool_call_id，但线性对话通常 i-1 就是
			const prevMsg = msgs[i - 1];
			let relatedCode = '';
			let relatedTool = '';

			if (prevMsg && prevMsg.owner === 'Assistant' && prevMsg.tool_use.length > 0) {
				// 假设图片是由最后一个工具调用产生的
				// (如果是并行调用，这里可能需要更复杂的匹配，先做简单版)
				const lastTool = prevMsg.tool_use[prevMsg.tool_use.length - 1];
				relatedCode = lastTool.args;
				relatedTool = lastTool.function_name;
			}

			msg.content.forEach((item) => {
				if ('ImageRef' in item) {
					images.push({
						src: `/api/image/${item.ImageRef[0]}`,
						alt: item.ImageRef[1],
						code: relatedCode,
						toolName: relatedTool
					});
				} else if ('ImageBin' in item) {
					images.push({
						src: `data:image/png;base64,${item.ImageBin[0]}`,
						alt: 'Base64 Image',
						code: relatedCode,
						toolName: relatedTool
					});
				}
			});
		});
		galleryImages = images.reverse();
	}
	$: editorTheme = $themeStore ? oneDark : [];
	$: if ($playgroundState) {
		activeTab = 'playground';
		selectedToolName = $playgroundState.toolName;
		try {
			const parsed = JSON.parse($playgroundState.args);
			inputArgs = typeof parsed === 'string' ? parsed : JSON.stringify(parsed, null, 2);
		} catch {
			inputArgs = $playgroundState.args;
		}
		playgroundState.set(null);
	}

	onMount(async () => {
		tools = await ChatService.getTools();
		if (tools.length > 0 && !selectedToolName) {
			selectedToolName = tools[0].name_for_model;
		}
	});

	async function handleRun() {
		isRunning = true;
		runResult = null;
		const result = await ChatService.runTool(selectedToolName, inputArgs);
		if (result) {
			runResult = result;
		}
		isRunning = false;
	}

	function handleGalleryClick(src: string) {
		dispatch('imageClick', src); // 派发事件给 +page.svelte
	}
</script>

<div class="flex h-full w-80 flex-col border-l border-base-300 bg-base-100 shadow-xl lg:w-96">
	<div class="flex-none bg-base-200 p-2">
		<div class="tabs-boxed tabs bg-base-100">
			<button
				class="tab flex-1"
				class:tab-active={activeTab === 'gallery'}
				on:click={() => (activeTab = 'gallery')}>Gallery</button
			>
			<button
				class="tab flex-1"
				class:tab-active={activeTab === 'playground'}
				on:click={() => (activeTab = 'playground')}>Playground</button
			>
		</div>
	</div>
	<div class="relative flex min-h-0 flex-1 flex-col">
		{#if activeTab === 'gallery'}
			<div class="flex-1 overflow-y-auto p-4">
				{#if galleryImages.length === 0}
					<div class="flex h-full items-center justify-center text-sm text-base-content/50">
						No images generated yet.
					</div>
				{:else}
					<div class="grid grid-cols-2 gap-2">
						{#each galleryImages as img}
							<div
								class="group relative aspect-square overflow-hidden rounded-lg border border-base-300 bg-base-200"
							>
								<img
									src={img.src}
									alt={img.alt}
									class="h-full w-full cursor-zoom-in object-cover transition-transform group-hover:scale-110"
									on:click={() => handleGalleryClick(img.src)}
								/>

								<div
									class="absolute top-1 right-1 flex gap-1 opacity-0 transition-opacity group-hover:opacity-100"
								>
									{#if img.code}
										<button
											class="btn btn-square border-0 bg-base-100/80 text-base-content shadow-sm backdrop-blur-sm btn-xs hover:bg-primary hover:text-white"
											on:click|stopPropagation={() => loadIntoPlayground(img)}
											title="Edit Code"
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
									{/if}

									<button
										class="btn btn-square border-0 bg-base-100/80 text-primary shadow-sm backdrop-blur-sm btn-xs hover:bg-white"
										on:click|stopPropagation={(e) => copyUUID(e, img.src)}
										title="Copy UUID"
									>
										<svg
											xmlns="http://www.w3.org/2000/svg"
											viewBox="0 0 20 20"
											fill="currentColor"
											class="h-3 w-3"
										>
											<path d="M8 3a1 1 0 011-1h2a1 1 0 110 2H9a1 1 0 01-1-1z" />
											<path
												d="M6 3a2 2 0 00-2 2v11a2 2 0 002 2h8a2 2 0 002-2V5a2 2 0 00-2-2 3 3 0 01-3 3H9a3 3 0 01-3-3z"
											/>
										</svg>
									</button>
								</div>
							</div>
						{/each}
					</div>
				{/if}
			</div>
		{/if}

		{#if activeTab === 'playground'}
			<div class="flex h-full flex-col gap-4 p-4">
				<div class="form-control flex-none">
					<label class="label text-xs font-bold">Select Tool</label>
					<select
						class="select-bordered select w-full select-sm"
						bind:value={selectedToolName}
						on:change={handleToolSwitch}
					>
						{#each tools as tool}
							<option value={tool.name_for_model}>{tool.name_for_human || tool.name_for_model}</option>
						{/each}
					</select>
				</div>
				<div
					class="form-control flex min-h-0 flex-1 flex-col overflow-hidden rounded-lg border border-base-300"
				>
					<div
						class="flex items-center justify-between border-b border-base-300 bg-base-200 px-3 py-2 text-xs font-bold"
					>
						<span>Input Parameters</span>
						<span class="font-mono text-[10px] opacity-50">
							{selectedToolName === 'js_interpreter' ? 'JavaScript' : 'JSON Schema'}
						</span>
					</div>
					<div class="relative min-h-0 flex-1 overflow-hidden">
						{#if selectedToolName === 'js_interpreter'}
							<CodeMirror
								bind:value={inputArgs}
								lang={javascript()}
								theme={editorTheme}
								styles={{
									'&': {
										height: '100%' /* 强制占满父容器高度 */,
										width: '100%'
									},
									'.cm-editor': {
										height: '100%' /* 编辑器本体也占满 */
									},
									'.cm-scroller': {
										overflow: 'auto' /* 允许滚动 */
									}
								}}
							/>
						{:else if currentToolDef && currentToolDef.parameters}
							<div class="h-full overflow-y-auto p-3">
								{#key selectedToolName}
									<SchemaForm schema={currentToolDef.parameters} bind:value={formValue} />
								{/key}
							</div>
						{:else}
							<textarea bind:value={inputArgs} class="textarea h-full w-full resize-none"
							></textarea>
						{/if}
					</div>
				</div>
				<div class="flex-none">
					<button class="btn btn-sm btn-primary" disabled={isRunning} on:click={handleRun}>
						{#if isRunning}
							<span class="loading loading-xs loading-spinner"></span>
						{:else}
							<svg
								xmlns="http://www.w3.org/2000/svg"
								viewBox="0 0 20 20"
								fill="currentColor"
								class="mr-1 h-4 w-4"
							>
								<path
									fill-rule="evenodd"
									d="M10 18a8 8 0 100-16 8 8 0 000 16zM9.555 7.168A1 1 0 008 8v4a1 1 0 001.555.832l3-2a1 1 0 000-1.664l-3-2z"
									clip-rule="evenodd"
								/>
							</svg>
						{/if}
						Run Tool
					</button>
				</div>

				{#if runResult}
					<div class="flex max-h-[40%] min-h-0 flex-none flex-col">
						<div class="divider my-1 text-xs">RESULT</div>
						<div
							class="flex-1 overflow-y-auto rounded-lg border border-base-300 bg-base-200/50 p-3 text-xs"
						>
							{#each runResult.content as item}
								{#if 'Text' in item}
									<div class="max-h-60 overflow-y-auto font-mono break-all whitespace-pre-wrap">
										{item.Text}
									</div>
								{:else if 'ImageRef' in item}
									<img
										src={`/api/image/${item.ImageRef[0]}`}
										alt="Result"
										class="mt-2 cursor-grab rounded border border-base-300 bg-base-100 active:cursor-grabbing"
										draggable="true"
										on:dragstart={(e) => {
											if (e.dataTransfer) {
												const markdown = `![Generated Image](/api/image/${item.ImageRef[0]})`;
												e.dataTransfer.setData('text/plain', markdown);
												e.dataTransfer.effectAllowed = 'copy';

												e.dataTransfer.setData(
													'DownloadURL',
													`image/png:image.png:${window.location.origin}/api/image/${item.ImageRef[0]}`
												);
											}
										}}
									/>
								{:else if 'ImageBin' in item}
									<img
										src={`data:image/png;base64,${item.ImageBin[0]}`}
										alt="Result"
										class="mt-2 rounded border border-base-300 bg-base-100"
										draggable="true"
										on:dragstart={(e) => {
											if (e.dataTransfer) {
												const markdown = `![Generated Image](/api/image/${item.ImageRef[0]})`;
												e.dataTransfer.setData('text/plain', markdown);
												e.dataTransfer.effectAllowed = 'copy';

												e.dataTransfer.setData(
													'DownloadURL',
													`image/png:image.png:${window.location.origin}/api/image/${item.ImageRef[0]}`
												);
											}
										}}
									/>
								{/if}
							{/each}
						</div>
					</div>
				{/if}
			</div>
		{/if}
	</div>
</div>
