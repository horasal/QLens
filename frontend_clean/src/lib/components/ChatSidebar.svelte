<script lang="ts">
	import { historyList, currentChat, processingChatIds } from '$lib/stores/chatStore';
	import * as ChatService from '$lib/services/chatService';
	import { _ } from 'svelte-i18n';
	import { onMount } from 'svelte';
	import { showSettings } from '$lib/stores/settingsStore';

	let isDark = false;

	onMount(() => {
		const savedTheme = localStorage.getItem('theme');
		if (savedTheme) {
			isDark = savedTheme === 'dim';
		} else {
			isDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
		}
	});

	$: {
		if (typeof document !== 'undefined') {
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
		<div class="mt-auto px-2">
			<button
				class="btn w-full justify-start gap-3 text-base-content/70 btn-ghost hover:bg-base-300 hover:text-base-content"
				on:click={() => $showSettings = true}
			>
				<svg
					xmlns="http://www.w3.org/2000/svg"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
					stroke-width="2"
					stroke-linecap="round"
					stroke-linejoin="round"
					class="h-5 w-5"
				>
					<path
						d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.1a2 2 0 0 1-1-1.74v-.47a2 2 0 0 1 1-1.74l.15-.1a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"
					></path>
					<circle cx="12" cy="12" r="3"></circle>
				</svg>
				<span class="font-medium">Settings</span>
			</button>
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
