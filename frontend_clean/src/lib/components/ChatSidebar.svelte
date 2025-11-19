<script lang="ts">
	import { historyList, currentChat, processingChatIds } from '$lib/stores/chatStore';
	import * as ChatService from '$lib/services/chatService';
	import { _ } from 'svelte-i18n';
	import { onMount } from 'svelte';

	let isDark = false;

	onMount(() => {
		const currentTheme = document.documentElement.getAttribute('data-theme');
		isDark = currentTheme === 'dim';
		document.documentElement.setAttribute('data-theme', savedTheme);
	});

	$: {
		if (typeof document !== 'undefined') {
			// 防止 SSR 报错
			const theme = isDark ? 'dim' : 'lofi';
			document.documentElement.setAttribute('data-theme', theme);
			localStorage.setItem('theme', theme);
		}
	}

	function handleDelete(e: MouseEvent, id: string) {
		e.stopPropagation();
		ChatService.deleteChat(id);
	}
</script>

<div class="drawer-side z-20">
	<label for="my-drawer" aria-label="close sidebar" class="drawer-overlay"></label>
	<div class="menu flex min-h-full w-80 flex-col bg-base-200 p-4 text-base-content">
		<div class="w-full flex-1 overflow-y-auto">
			<ul class="menu w-full px-0">
				<li class="mb-2 w-full min-w-1">
					<button class="btn btn-primary" on:click={ChatService.startNewChat}>
						+ {$_('new_chat')}
					</button>
				</li>
				{#each $historyList as chat (chat.id)}
					<li
						class:active={$currentChat?.id === chat.id}
						class="w-full overflow-hidden no-underline"
					>
						<div
							on:click={() => ChatService.loadChat(chat.id)}
							class="group flex w-full cursor-pointer items-center justify-between no-underline hover:no-underline"
						>
							<button
								class="btn btn-circle text-error/70 no-underline opacity-0 btn-ghost transition-opacity btn-xs group-hover:opacity-100 hover:bg-error/20"
								on:click={(e) => handleDelete(e, chat.id)}
								title="Delete"
							>
								✕
							</button>

							<div class="mx-2 min-w-0 flex-1 overflow-hidden">
								<p class="truncate">{chat.summary || $_('no_title')}</p>
								<span class="text-xs text-base-content/50 no-underline">
									{new Date(chat.date).toLocaleString()}
								</span>
							</div>

							{#if $processingChatIds.has(chat.id)}
								<span class="loading ml-2 loading-xs loading-spinner text-primary"></span>
							{/if}
						</div>
					</li>
				{/each}
			</ul>
		</div>
		<div class="mt-2 border-t border-base-300 pt-4">
			<label
				class="flex cursor-pointer items-center gap-2 rounded-lg px-4 py-2 transition-colors hover:bg-base-300"
			>
				<svg
					xmlns="http://www.w3.org/2000/svg"
					width="20"
					height="20"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
					stroke-width="2"
					stroke-linecap="round"
					stroke-linejoin="round"
					><circle cx="12" cy="12" r="5" /><path
						d="M12 1v2M12 21v2M4.2 4.2l1.4 1.4M18.4 18.4l1.4 1.4M1 12h2M21 12h2M4.2 19.8l1.4-1.4M18.4 5.6l1.4-1.4"
					/></svg
				>

				<input type="checkbox" class="theme-controller toggle" bind:checked={isDark} />

				<svg
					xmlns="http://www.w3.org/2000/svg"
					width="20"
					height="20"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
					stroke-width="2"
					stroke-linecap="round"
					stroke-linejoin="round"
					><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"></path></svg
				>
			</label>
		</div>
	</div>
</div>
