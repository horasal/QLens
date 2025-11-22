<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/stores';
	import { _ } from 'svelte-i18n';
	import { isLoading as i18nLoading } from 'svelte-i18n';
	import { initI18n } from '$lib/i8n';

	// 引入组件
	import ChatSidebar from '$lib/components/ChatSidebar.svelte';
	import MessageList from '$lib/components/MessageList.svelte';
	import ChatInput from '$lib/components/ChatInput.svelte';
	import ErrorToast from '$lib/components/ErrorToast.svelte';
	import ImageModal from '$lib/components/ImageModal.svelte';
	import ArtifactPanel from '$lib/components/ArtifactPanel.svelte';

	// 引入服务和状态
	import * as ChatService from '$lib/services/chatService';
	import { isDragging, isLoading as isGlobalLoading } from '$lib/stores/chatStore';
	import SettingsModal from '$lib/components/SettingsModal.svelte';
	import { showSettings, showArtifacts } from '$lib/stores/settingsStore';

	initI18n();

	// 绑定子组件实例，用于调用方法
	let chatInputComponent: ChatInput;
	let modalImageUrl: string | null = null;

	onMount(async () => {
		await ChatService.init();
		// URL 参数处理
		const urlId = $page.url.searchParams.get('id');
		if (urlId) {
			ChatService.loadChat(urlId);
		}
	});

	// --- 图片预览 Modal ---
	function showImageModal(src: string) {
		modalImageUrl = src;
	}

	// --- 拖拽处理 ---
	function handleDragOver(e: DragEvent) {
		e.preventDefault();
		isDragging.set(true);
	}

	function handleDragLeave(e: DragEvent) {
		const currentTarget = e.currentTarget as HTMLElement;
		if (!e.relatedTarget || !currentTarget.contains(e.relatedTarget as Node)) {
			isDragging.set(false);
		}
	}

	function handleDrop(e: DragEvent) {
		e.preventDefault();
		isDragging.set(false);
		// 调用 Input 组件的方法添加文件
		chatInputComponent?.addFiles(e.dataTransfer?.files ?? null);
	}
</script>

{#if $i18nLoading}
	<div
		class="fixed inset-0 z-50 flex flex-col items-center justify-center bg-base-100 text-primary"
	>
		<span class="loading loading-lg scale-150 loading-infinity"></span>
		<p class="mt-6 animate-pulse text-sm font-medium tracking-widest uppercase opacity-70">
			System Initializing...
		</p>
	</div>
{:else}
	<ImageModal src={modalImageUrl} on:close={() => (modalImageUrl = null)} />
	<SettingsModal bind:show={$showSettings} />

	<div class="drawer lg:drawer-open">
		<input id="my-drawer" type="checkbox" class="drawer-toggle" />

		<ChatSidebar />

		<div
			class="drawer-content flex h-screen flex-row overflow-hidden"
			role="region"
			on:dragover={handleDragOver}
			on:dragleave={handleDragLeave}
			on:drop={handleDrop}
		>
			<ErrorToast />

			<div class="relative flex min-w-0 flex-1 flex-col transition-all duration-300">
				<div class="navbar min-h-[3rem] border-b border-base-300 bg-base-100 p-0 pr-2">
					<div class="flex-none lg:hidden">
						<label for="my-drawer" class="btn btn-square btn-ghost btn-sm">
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

					<div class="flex-1 px-2">
						<a href="/" class="btn text-lg font-bold btn-ghost btn-sm lg:hidden">QLens</a>
					</div>

					<button
						class="btn gap-2 btn-ghost btn-sm"
						class:btn-active={$showArtifacts}
						on:click={() => ($showArtifacts = !$showArtifacts)}
						title="Toggle Artifacts Panel"
					>
						<svg
							xmlns="http://www.w3.org/2000/svg"
							fill="none"
							viewBox="0 0 24 24"
							stroke-width="1.5"
							stroke="currentColor"
							class="h-5 w-5"
						>
							<path
								stroke-linecap="round"
								stroke-linejoin="round"
								d="M3.75 6A2.25 2.25 0 016 3.75h2.25A2.25 2.25 0 0110.5 6v2.25a2.25 2.25 0 01-2.25 2.25H6a2.25 2.25 0 01-2.25-2.25V6zM3.75 15.75A2.25 2.25 0 016 13.5h2.25a2.25 2.25 0 012.25 2.25V18a2.25 2.25 0 01-2.25 2.25H6A2.25 2.25 0 013.75 18v-2.25zM13.5 6a2.25 2.25 0 012.25-2.25H18A2.25 2.25 0 0120.25 6v2.25A2.25 2.25 0 0118 10.5h-2.25a2.25 2.25 0 01-2.25-2.25V6zM13.5 15.75a2.25 2.25 0 012.25-2.25H18A2.25 2.25 0 0120.25 6v2.25A2.25 2.25 0 0118 10.5h-2.25a2.25 2.25 0 01-2.25-2.25V6z"
							/>
						</svg>
						<span class="hidden sm:inline">Artifacts</span>
					</button>
				</div>

				<MessageList on:imageClick={(e) => showImageModal(e.detail)} />

				<ChatInput bind:this={chatInputComponent} />

				{#if $isDragging}
					<div
						class="pointer-events-none absolute inset-0 z-50 m-4 flex items-center justify-center rounded-xl border-4 border-dashed border-primary bg-primary/20 backdrop-blur-sm"
					>
						<span class="text-2xl font-bold text-primary drop-shadow-md"
							>{$_('drag_zone') || 'Drop files here'}</span
						>
					</div>
				{/if}
			</div>

			{#if $showArtifacts}
				<ArtifactPanel on:imageClick={(e) => showImageModal(e.detail)} />
			{/if}
		</div>
	</div>
{/if}
