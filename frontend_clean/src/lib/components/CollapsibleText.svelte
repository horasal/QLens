<script lang="ts">
    import MarkdownBlock from './MarkdownBlock.svelte';
    import { createEventDispatcher } from 'svelte';

    export let text: string;
    export let maxLines: number = 15;
    export let maxChars: number = 500; // 增加字符限制，防止单行过长

    const dispatch = createEventDispatcher();

    let expanded = false;
    let isOverflowing = false;
    let container: HTMLElement;

    $: {
        const lines = text.split('\n').length;
        isOverflowing = lines > maxLines || text.length > maxChars;
    }

    function toggle() {
        expanded = !expanded;
    }

    // 转发 MarkdownBlock 的图片点击事件
    function forwardImageClick(e: CustomEvent) {
        dispatch('imageClick', e.detail);
    }
</script>

<div class="relative group/collapsible">
    <div
        class="whitespace-pre-wrap transition-all duration-300 ease-in-out overflow-hidden"
        class:max-h-[24rem]={isOverflowing && !expanded}
        class:max-h-none={!isOverflowing || expanded}
    >
        <MarkdownBlock content={text} on:imageClick={forwardImageClick} />
    </div>

    {#if isOverflowing && !expanded}
        <div class="absolute bottom-0 left-0 right-0 h-24 bg-gradient-to-t from-base-100/90 to-transparent flex items-end justify-center pb-2 pt-12 transition-opacity duration-300">
            <button
                class="btn btn-xs btn-neutral gap-1 shadow-md hover:scale-105 transition-transform"
                on:click={toggle}
            >
                <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor" class="w-3 h-3">
                  <path fill-rule="evenodd" d="M10 3a.75.75 0 01.75.75v10.638l3.96-4.158a.75.75 0 111.08 1.04l-5.25 5.5a.75.75 0 01-1.08 0l-5.25-5.5a.75.75 0 111.08-1.04l3.96 4.158V3.75A.75.75 0 0110 3z" clip-rule="evenodd" />
                </svg>
                Show More
            </button>
        </div>
    {:else if isOverflowing && expanded}
        <div class="flex justify-center mt-2 opacity-0 group-hover/collapsible:opacity-100 transition-opacity duration-200">
            <button
                class="btn btn-xs btn-ghost text-base-content/50 gap-1"
                on:click={toggle}
            >
                <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor" class="w-3 h-3">
                  <path fill-rule="evenodd" d="M10 17a.75.75 0 01-.75-.75V5.612L5.29 9.77a.75.75 0 01-1.08-1.04l5.25-5.5a.75.75 0 011.08 0l5.25 5.5a.75.75 0 11-1.08 1.04l-3.96-4.158V16.25A.75.75 0 0110 17z" clip-rule="evenodd" />
                </svg>
                Show Less
            </button>
        </div>
    {/if}
</div>
