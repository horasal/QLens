<script lang="ts">
	import { currentChat } from '$lib/stores/chatStore';
	import * as ChatService from '$lib/services/chatService';
	import { onMount } from 'svelte';
	import type { ToolDescription, Message, MessageContent } from '$lib/types';
	import { playgroundState } from '$lib/stores/artifactStore';
	import { createEventDispatcher } from 'svelte';
	import { toasts } from '$lib/stores/chatStore';
	import { themeStore } from '$lib/stores/themeStore';
	import CodeMirror from 'svelte-codemirror-editor';
	import { javascript } from '@codemirror/lang-javascript';
	import { json } from '@codemirror/lang-json';
	import { oneDark } from '@codemirror/theme-one-dark';
	import SmartForm from './SmartForm.svelte'; // 使用新的表单组件
	import { EditorView } from '@codemirror/view';
	import { getApiBase } from '$lib/services/baseUrl';

	const dispatch = createEventDispatcher();

	let activeTab: 'gallery' | 'playground' = 'gallery';
	let tools: ToolDescription[] = [];
	let selectedToolName = '';
	let isRunning = false;
	let runResult: Message | null = null;
	let isExpanded = false;

	type ToolDraft = {
		argsJsonString: string; // CodeMirror 的真值
		argsObject: any; // SmartForm 的真值
		mode: 'code' | 'form'; // 当前偏好模式
	};
	// 缓存：toolName -> Draft
	let toolDrafts: Record<string, ToolDraft> = {};

	// 获取当前工具的草稿，如果不存在则初始化
	function getDraft(toolName: string): ToolDraft {
		if (!toolDrafts[toolName]) {
			// 默认初始化为空对象或空字符串
			const isJs = toolName === 'js_interpreter';
			toolDrafts[toolName] = {
				argsJsonString: isJs ? '' : '{}',
				argsObject: {},
				mode: isJs ? 'code' : 'form'
			};
		}
		return toolDrafts[toolName];
	}

	// 当前选中的工具的草稿（响应式）
	$: currentDraft = selectedToolName ? getDraft(selectedToolName) : null;
	$: currentToolDef = tools.find((t) => t.name_for_model === selectedToolName);

	// 当 CodeMirror 文本改变时
	function handleCodeChange(newVal: string) {
		if (!currentDraft) return;
		currentDraft.argsJsonString = newVal;

		// 尝试同步给 Form (如果是 JSON)
		if (selectedToolName !== 'js_interpreter') {
			try {
				currentDraft.argsObject = JSON.parse(newVal);
			} catch {
				// 解析失败不强行同步，允许用户输入中间状态
			}
		}
	}

	// 当 SmartForm 对象改变时
	function handleFormChange(newVal: any) {
		if (!currentDraft) return;
		currentDraft.argsObject = newVal;
		// 同步回 CodeMirror 文本
		currentDraft.argsJsonString = JSON.stringify(newVal, null, 2);
	}

	// --- Gallery Logic ---
	type GalleryItem = {
		src: string;
		alt: string;
		code: string;
		toolName: string;
		msgId: string;
	};
	let galleryImages: GalleryItem[] = [];
	let editorRefreshKey = 0;

	$: if ($currentChat) {
		const images: GalleryItem[] = [];
		const msgs = $currentChat.messages;

		msgs.forEach((msg, i) => {
			// 兼容逻辑：检查 role 是否为 tool (对象 or 字符串)
			const isTool =
				typeof msg.owner === 'string' ? msg.owner === 'Tools' : msg.owner.role === 'tool';
			if (!isTool) return;

			// 尝试寻找对应的调用参数
			// 1. 如果是新架构，msg.owner 包含 tool_call_id
			// 2. 如果是旧架构，回溯上一条 Assistant
			let relatedCode = '';
			let relatedToolName = '';

			// 新架构：通过 tool_call_id 查找
			if (typeof msg.owner === 'object' && msg.owner.role === 'tool') {
				const callId = msg.owner.tool_call_id;
				// 在之前的消息里找 tool_use.use_id === callId
				// 倒序查找最近的
				for (let j = i - 1; j >= 0; j--) {
					const m = msgs[j];
					const uses = m.tool_use || [];
					const found = uses.find((u) => u.use_id === callId);
					if (found) {
						relatedCode = found.args;
						relatedToolName = found.function_name;
						break;
					}
				}
			} else {
				// 旧架构回退逻辑 (Prev msg is assistant)
				const prevMsg = msgs[i - 1];
				if (
					prevMsg &&
					(prevMsg.owner === 'Assistant' ||
						(typeof prevMsg.owner === 'object' && prevMsg.owner.role === 'assistant'))
				) {
					if (prevMsg.tool_use.length > 0) {
						const last = prevMsg.tool_use[prevMsg.tool_use.length - 1];
						relatedCode = last.args;
						relatedToolName = last.function_name;
					}
				}
			}

			msg.content.forEach((item) => {
				if ('ImageRef' in item) {
					images.push({
						src: `${getApiBase()}/api/image/${item.ImageRef[0]}`,
						alt: item.ImageRef[1] || 'Image',
						code: relatedCode,
						toolName: relatedToolName,
						msgId: msg.id
					});
				} else if ('ImageBin' in item) {
					images.push({
						src: `data:image/png;base64,${item.ImageBin[0]}`,
						alt: 'Base64',
						code: relatedCode,
						toolName: relatedToolName,
						msgId: msg.id
					});
				}
			});
		});
		galleryImages = images.reverse();
	}

	function copyUUID(e: MouseEvent, src: string) {
		const uuid = src.split('/').pop();
		if (uuid) {
			navigator.clipboard.writeText(uuid);
			toasts.show('UUID Copied!', 'success', 1000);
		}
	}

	function loadIntoPlayground(item: GalleryItem) {
		activeTab = 'playground';
		// 切换工具
		if (item.toolName) selectedToolName = item.toolName;

		// 强制覆写该工具的 Draft
		const d = getDraft(selectedToolName);
		d.argsJsonString = item.code;

		if (selectedToolName !== 'js_interpreter') {
			try {
				d.argsObject = JSON.parse(item.code);
				// 如果 JSON 解析成功，格式化一下代码
				d.argsJsonString = JSON.stringify(d.argsObject, null, 2);
			} catch {
				d.argsObject = {};
			}
		}

		// 触发更新
		toolDrafts = { ...toolDrafts };
	}

	$: editorTheme = $themeStore ? oneDark : [];

	// 监听从聊天界面点击 "Edit" 传过来的 State
	$: if ($playgroundState) {
		activeTab = 'playground';
		const { toolName, args } = $playgroundState;
		selectedToolName = toolName;

		const d = getDraft(toolName);
		d.argsJsonString = args;
		if (toolName !== 'js_interpreter') {
			try {
				d.argsObject = JSON.parse(args);
				d.argsJsonString = JSON.stringify(d.argsObject, null, 2);
			} catch {}
		}
		toolDrafts = { ...toolDrafts };
		editorRefreshKey++;
		playgroundState.set(null);
	}

	onMount(async () => {
		tools = await ChatService.getTools();
		if (tools.length > 0 && !selectedToolName) {
			selectedToolName = tools[0].name_for_model;
		}
	});

	async function handleRun() {
		if (!currentDraft) return;
		isRunning = true;
		runResult = null;
		try {
			const result = await ChatService.runTool(selectedToolName, currentDraft.argsJsonString);
			if (result) runResult = result;
		} catch (e) {
			console.error(e);
		} finally {
			isRunning = false;
		}
	}

	function handleGalleryClick(src: string) {
		dispatch('imageClick', src);
	}
</script>

<div
	class="flex h-full flex-col border-l border-base-300 bg-base-100 shadow-xl transition-all duration-300 ease-in-out"
	class:w-80={!isExpanded}
	class:lg:w-96={!isExpanded}
	class:w-[50vw]={isExpanded}
>
	<div class="flex-none bg-base-200 p-2">
		<div class="tabs-boxed tabs bg-base-100 tabs-sm">
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
	<button
		class="btn btn-square btn-ghost btn-sm"
		on:click={() => (isExpanded = !isExpanded)}
		title={isExpanded ? 'Collapse' : 'Expand'}
	>
		{#if isExpanded}
			<svg
				xmlns="http://www.w3.org/2000/svg"
				viewBox="0 0 20 20"
				fill="currentColor"
				class="h-4 w-4"
			>
				<path
					fill-rule="evenodd"
					d="M10.21 14.77a.75.75 0 01.02-1.06L14.16 10l-3.93-3.71a.75.75 0 011.04-1.08l4.5 4.25a.75.75 0 010 1.08l-4.5 4.25a.75.75 0 01-1.06-.02z"
					clip-rule="evenodd"
				/>
				<path
					fill-rule="evenodd"
					d="M4.21 14.77a.75.75 0 01.02-1.06L8.16 10 4.23 6.29a.75.75 0 011.04-1.08l4.5 4.25a.75.75 0 010 1.08l-4.5 4.25a.75.75 0 01-1.06-.02z"
					clip-rule="evenodd"
				/>
			</svg>
		{:else}
			<svg
				xmlns="http://www.w3.org/2000/svg"
				viewBox="0 0 20 20"
				fill="currentColor"
				class="h-4 w-4"
			>
				<path
					fill-rule="evenodd"
					d="M15.79 14.77a.75.75 0 01-1.06.02l-4.5-4.25a.75.75 0 010-1.08l4.5-4.25a.75.75 0 111.04 1.08L11.84 10l3.93 3.71a.75.75 0 01.02 1.06z"
					clip-rule="evenodd"
				/>
				<path
					fill-rule="evenodd"
					d="M9.79 14.77a.75.75 0 01-1.06.02l-4.5-4.25a.75.75 0 010-1.08l4.5-4.25a.75.75 0 111.04 1.08L5.84 10l3.93 3.71a.75.75 0 01.02 1.06z"
					clip-rule="evenodd"
				/>
			</svg>
		{/if}
	</button>
	<div class="relative flex min-h-0 flex-1 flex-col">
		{#if activeTab === 'gallery'}
			<div class="flex-1 overflow-y-auto p-4">
				{#if galleryImages.length === 0}
					<div class="flex h-full items-center justify-center text-sm text-base-content/50">
						No images generated yet.
					</div>
				{:else}
					<div class="grid grid-cols-2 gap-2">
						{#each galleryImages as img (img.src)}
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
											class="btn btn-square border-0 bg-base-100/80 backdrop-blur-sm btn-xs hover:bg-primary hover:text-white"
											on:click|stopPropagation={() => loadIntoPlayground(img)}
											title="Edit Code"
										>
											<svg
												xmlns="http://www.w3.org/2000/svg"
												viewBox="0 0 20 20"
												fill="currentColor"
												class="h-3 w-3"
												><path
													d="M5.433 13.917l1.262-3.155A4 4 0 017.58 9.42l6.92-6.918a2.121 2.121 0 013 3l-6.92 6.918c-.383.383-.84.685-1.343.886l-3.154 1.262a.5.5 0 01-.65-.65z"
												/></svg
											>
										</button>
									{/if}
									<button
										class="btn btn-square border-0 bg-base-100/80 text-primary backdrop-blur-sm btn-xs hover:bg-white"
										on:click|stopPropagation={(e) => copyUUID(e, img.src)}
										title="Copy UUID"
									>
										<svg
											xmlns="http://www.w3.org/2000/svg"
											viewBox="0 0 20 20"
											fill="currentColor"
											class="h-3 w-3"
											><path d="M8 3a1 1 0 011-1h2a1 1 0 110 2H9a1 1 0 01-1-1z" /><path
												d="M6 3a2 2 0 00-2 2v11a2 2 0 002 2h8a2 2 0 002-2V5a2 2 0 00-2-2 3 3 0 01-3 3H9a3 3 0 01-3-3z"
											/></svg
										>
									</button>
								</div>
							</div>
						{/each}
					</div>
				{/if}
			</div>
		{/if}

		{#if activeTab === 'playground' && currentDraft}
			<div class="flex h-full flex-col gap-3 p-4">
				<div class="form-control flex-none">
					<label class="label py-1 text-xs font-bold text-base-content/70">Select Tool</label>
					<select
						class="select-bordered select w-full select-sm text-xs"
						bind:value={selectedToolName}
					>
						{#each tools as tool}
							<option value={tool.name_for_model}
								>{tool.name_for_human || tool.name_for_model}</option
							>
						{/each}
					</select>
				</div>

				<div
					class="flex min-h-0 flex-1 flex-col overflow-hidden rounded-lg border border-base-300 bg-base-100"
				>
					<div
						class="flex items-center justify-between border-b border-base-300 bg-base-200 px-3 py-1.5"
					>
						<span class="text-xs font-bold opacity-70">Input Parameters</span>

						{#if selectedToolName !== 'js_interpreter'}
							<div class="join">
								<button
									class="btn join-item btn-xs {currentDraft.mode === 'form'
										? 'btn-active btn-neutral'
										: ''}"
									on:click={() => (currentDraft.mode = 'form')}>Form</button
								>
								<button
									class="btn join-item btn-xs {currentDraft.mode === 'code'
										? 'btn-active btn-neutral'
										: ''}"
									on:click={() => (currentDraft.mode = 'code')}>Code</button
								>
							</div>
						{:else}
							<span class="font-mono text-[10px] opacity-50">JavaScript</span>
						{/if}
					</div>
					{#key editorRefreshKey}
						<div class="flex-1 overflow-hidden bg-base-100">
							{#if selectedToolName === 'js_interpreter'}
								<CodeMirror
									bind:value={currentDraft.argsJsonString}
									on:change={(e) => handleCodeChange(e.detail)}
									lang={javascript()}
									theme={editorTheme}
									extensions={[EditorView.lineWrapping]}
									styles={{
										'&': { height: '100%' } /* 强制编辑器占满父容器高度 */,
										'.cm-scroller': { overflow: 'auto' } /* 强制启用滚动 */
									}}
								/>
							{:else if currentDraft.mode === 'form' && currentToolDef && currentToolDef.parameters}
								<div class="h-full overflow-y-auto p-3">
									<SmartForm
										schema={currentToolDef.parameters}
										value={currentDraft.argsObject}
										on:change={(e) => handleFormChange(e.detail)}
									/>
								</div>
							{:else}
								<CodeMirror
									bind:value={currentDraft.argsJsonString}
									on:change={(e) => handleCodeChange(e.detail)}
									lang={json()}
									theme={editorTheme}
									extensions={[EditorView.lineWrapping]}
									styles={{
										'&': { height: '100%', width: '100%' },
										'.cm-scroller': { overflow: 'auto' }
									}}
								/>
							{/if}
						</div>
					{/key}
				</div>
				<div class="flex-none">
					<button class="btn w-full btn-sm btn-primary" disabled={isRunning} on:click={handleRun}>
						{#if isRunning}
							<span class="loading loading-xs loading-spinner"></span>
						{:else}
							<svg
								xmlns="http://www.w3.org/2000/svg"
								viewBox="0 0 20 20"
								fill="currentColor"
								class="mr-1 h-4 w-4"
								><path
									fill-rule="evenodd"
									d="M10 18a8 8 0 100-16 8 8 0 000 16zM9.555 7.168A1 1 0 008 8v4a1 1 0 001.555.832l3-2a1 1 0 000-1.664l-3-2z"
									clip-rule="evenodd"
								/></svg
							>
						{/if}
						Run Tool
					</button>
				</div>
				{#if runResult}
					<div class="flex max-h-[40%] min-h-0 flex-none flex-col border-t border-base-300 pt-2">
						<div class="mb-1 flex items-center justify-between px-1">
							<span class="text-[10px] font-bold text-base-content/50 uppercase"
								>Execution Result</span
							>
							<button
								class="btn h-5 min-h-0 text-[10px] font-normal text-base-content/50 btn-ghost btn-xs"
								on:click={() => (runResult = null)}>Clear</button
							>
						</div>

						<div class="flex-1 overflow-y-auto rounded-md bg-base-200 p-2 text-xs">
							{#each runResult.content as item}
								{#if 'Text' in item}
									<div class="mb-2 font-mono break-all whitespace-pre-wrap">{item.Text}</div>
								{:else if 'ImageRef' in item || 'ImageBin' in item}
									<div
										class="group relative mb-2 inline-block max-w-full overflow-hidden rounded-lg border border-base-300 bg-base-100"
									>
										<img
											src={'ImageRef' in item
												? `${getApiBase()}/api/image/${item.ImageRef[0]}`
												: `data:image/png;base64,${item.ImageBin[0]}`}
											alt="Result"
											class="max-h-60 w-auto cursor-zoom-in object-contain transition-transform"
											on:click={() =>
												handleGalleryClick(
													'ImageRef' in item
														? `${getApiBase()}/api/image/${item.ImageRef[0]}`
														: `data:image/png;base64,${item.ImageBin[0]}`
												)}
											draggable="true"
											on:dragstart={(e) => {
												// 保留原本的拖拽逻辑
												if (e.dataTransfer) {
													const src =
														'ImageRef' in item
															? `${getApiBase()}/api/image/${item.ImageRef[0]}`
															: `data:image/png;base64,${item.ImageBin[0]}`;
													const markdown = `![Generated Image](${src})`;
													e.dataTransfer.setData('text/plain', markdown);
													e.dataTransfer.effectAllowed = 'copy';
												}
											}}
										/>

										<div
											class="absolute top-1 right-1 flex gap-1 opacity-0 transition-opacity group-hover:opacity-100"
										>
											{#if 'ImageRef' in item}
												<button
													class="btn btn-square border-0 bg-base-100/80 text-primary shadow-sm backdrop-blur-sm btn-xs hover:bg-white"
													on:click|stopPropagation={(e) =>
														copyUUID(e, `${getApiBase()}/api/image/${item.ImageRef[0]}`)}
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
											{/if}

											<a
												href={'ImageRef' in item
													? `${getApiBase()}/api/image/${item.ImageRef[0]}`
													: `data:image/png;base64,${item.ImageBin[0]}`}
												target="_blank"
												class="btn btn-square border-0 bg-base-100/80 text-base-content/70 shadow-sm backdrop-blur-sm btn-xs hover:bg-primary hover:text-white"
												title="Open in New Tab"
												on:click|stopPropagation
											>
												<svg
													xmlns="http://www.w3.org/2000/svg"
													viewBox="0 0 20 20"
													fill="currentColor"
													class="h-3 w-3"
												>
													<path
														fill-rule="evenodd"
														d="M4.25 5.5a.75.75 0 00-.75.75v8.5c0 .414.336.75.75.75h8.5a.75.75 0 00.75-.75v-4a.75.75 0 011.5 0v4A2.25 2.25 0 0112.75 17h-8.5A2.25 2.25 0 012 14.75v-8.5A2.25 2.25 0 014.25 4h5a.75.75 0 010 1.5h-5z"
														clip-rule="evenodd"
													/>
													<path
														fill-rule="evenodd"
														d="M6.194 12.753a.75.75 0 001.06.053L16.5 4.44v2.81a.75.75 0 001.5 0v-4.5a.75.75 0 00-.75-.75h-4.5a.75.75 0 000 1.5h2.553l-9.056 8.194a.75.75 0 00-.053 1.06z"
														clip-rule="evenodd"
													/>
												</svg>
											</a>
										</div>
									</div>
								{/if}
							{/each}
						</div>
					</div>
				{/if}
			</div>
		{/if}
	</div>
</div>

<style>
	:global(.codemirror-wrapper) {
		height: 100% !important;
	}

	:global(.cm-editor) {
		height: 100% !important;
		max-height: 100% !important;
	}

	/* 确保滚动条容器能正常工作 */
	:global(.cm-scroller) {
		overflow: auto !important;
		height: 100% !important;
	}

	/* 可选：美化一下滚动条，让它在窄屏下更明显 */
	:global(.cm-scroller::-webkit-scrollbar) {
		width: 8px;
		height: 8px;
	}
	:global(.cm-scroller::-webkit-scrollbar-track) {
		background: transparent;
	}
	:global(.cm-scroller::-webkit-scrollbar-thumb) {
		background-color: #cbd5e1; /* base-300 */
		border-radius: 4px;
	}
	:global(.cm-scroller::-webkit-scrollbar-thumb:hover) {
		background-color: #94a3b8;
	}
</style>
