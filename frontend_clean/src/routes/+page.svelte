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

	// 引入服务和状态
	import * as ChatService from '$lib/services/chatService';
	import { isDragging, isLoading as isGlobalLoading } from '$lib/stores/chatStore';
	import SettingsModal from '$lib/components/SettingsModal.svelte';
	import { showSettings } from '$lib/stores/settingsStore';

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
			class="relative drawer-content flex h-screen flex-col"
			role="region"
			on:dragover={handleDragOver}
			on:dragleave={handleDragLeave}
			on:drop={handleDrop}
		>
			<ErrorToast />

			<div class="navbar bg-base-100 lg:hidden">
				<div class="flex-none">
					<label for="my-drawer" class="btn btn-square btn-ghost">
						<svg
							xmlns="http://www.w3.org/2000/svg"
							fill="none"
							viewBox="0 0 24 24"
							class="inline-block h-5 w-5 stroke-current"
						>
							<path
								stroke-linecap="round"
								stroke-linejoin="round"
								stroke-width="2"
								d="M4 6h16M4 12h16M4 18h16"
							></path>
						</svg>
					</label>
				</div>
				<div class="flex-1">
					<a href="/" class="btn text-xl btn-ghost">QLens</a>
				</div>
			</div>

			<MessageList on:imageClick={(e) => showImageModal(e.detail)} />

			<ChatInput bind:this={chatInputComponent} />

			{#if $isDragging}
				<div
					class="pointer-events-none absolute inset-0 z-50 flex items-center justify-center border-4 border-dashed border-primary bg-primary/20"
				>
					<span class="text-2xl font-bold text-primary">{$_('drag_zone') || 'Drop files here'}</span
					>
				</div>
			{/if}
		</div>
	</div>
{/if}
