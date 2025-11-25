<script lang="ts">
	import { renderMarkdown } from '$lib/markdownRenderer';

	export let content: string = '';
	export let disableImages: boolean = false;

	let renderedHtml: string = '';
	let lastContent: string = '';
	let updateScheduled = false;

	$: if (content !== lastContent || disableImages) {
		scheduleUpdate();
	}

	function scheduleUpdate() {
		if (updateScheduled) return;
		updateScheduled = true;

		requestAnimationFrame(() => {
			renderedHtml = renderMarkdown(content, { disableImages });
			lastContent = content;
			updateScheduled = false;
		});
	}

	function handleBodyClick(e: MouseEvent) {
		// 这里处理之前的 handleMarkdownClick 逻辑
		// 派发事件给父组件处理图片点击
		const target = e.target as HTMLElement;
		if (target.tagName === 'IMG') {
			const src = target.getAttribute('src');
			if (src) {
				dispatch('imageClick', src);
			}
		}
	}

	import { createEventDispatcher } from 'svelte';
	const dispatch = createEventDispatcher();
</script>

<div class="markdown-body prose max-w-none" on:click={handleBodyClick}>
	{@html renderedHtml}
</div>
