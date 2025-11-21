<script lang="ts">
	import { fade, scale } from 'svelte/transition';
	import { quintOut } from 'svelte/easing';
	import { createEventDispatcher } from 'svelte';

	export let src: string | null = null;
	export let alt: string = 'Image preview';

	const dispatch = createEventDispatcher();

	function close() {
		// 派发关闭事件，父组件负责把 src 设为 null 来隐藏
		dispatch('close');
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') close();
	}
</script>

<svelte:window on:keydown={handleKeydown} />

{#if src}
	<div
		class="fixed inset-0 z-[999] flex items-center justify-center bg-base-100/80 backdrop-blur-md transition-all"
		transition:fade={{ duration: 200 }}
		on:click|self={close}
		role="button"
		tabindex="0"
	>
		<div
			class="relative flex items-center justify-center p-4 outline-none"
			transition:scale={{ duration: 300, opacity: 0.5, start: 0.9, easing: quintOut }}
		>
			<button
				class="btn absolute -top-10 -right-2 btn-circle text-base-content btn-ghost btn-sm hover:bg-base-content/10 sm:top-0 sm:-right-10"
				on:click={close}
			>
				<svg
					xmlns="http://www.w3.org/2000/svg"
					class="h-6 w-6"
					fill="none"
					viewBox="0 0 24 24"
					stroke="currentColor"
				>
					<path
						stroke-linecap="round"
						stroke-linejoin="round"
						stroke-width="2"
						d="M6 18L18 6M6 6l12 12"
					/>
				</svg>
			</button>

			<img
				{src}
				{alt}
				class="max-h-[90vh] max-w-[90vw] rounded-lg object-contain shadow-2xl"
				on:click|stopPropagation
			/>

			<div
				class="absolute right-0 -bottom-12 left-0 flex justify-center gap-4 opacity-0 transition-opacity hover:opacity-100"
			>
				<a href={src} download="image.png" class="btn btn-outline btn-sm" on:click|stopPropagation>
					Download
				</a>
			</div>
		</div>
	</div>
{/if}
