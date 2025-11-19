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
		class="fixed inset-0 z-[999] flex items-center justify-center bg-black/90 backdrop-blur-sm transition-all"
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
				class="absolute -right-2 -top-10 btn btn-circle btn-sm btn-ghost text-white hover:bg-white/20 sm:-right-10 sm:top-0"
				on:click={close}
			>
				<svg xmlns="http://www.w3.org/2000/svg" class="h-6 w-6" fill="none" viewBox="0 0 24 24" stroke="currentColor">
					<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
				</svg>
			</button>

			<img
				{src}
				{alt}
				class="max-h-[90vh] max-w-[90vw] rounded-lg object-contain shadow-2xl drop-shadow-2xl"
                on:click|stopPropagation
			/>

            <div class="absolute -bottom-12 left-0 right-0 flex justify-center gap-4 opacity-0 transition-opacity hover:opacity-100">
                <a href={src} download="image.png" class="btn btn-sm btn-outline text-white border-white hover:bg-white hover:text-black" on:click|stopPropagation>
                    Download
                </a>
            </div>
		</div>
	</div>
{/if}
