<script lang="ts">
    import { onMount, onDestroy, createEventDispatcher } from 'svelte';
    import { JSONEditor } from '@json-editor/json-editor';

    export let schema: any;
    export let value: any;

    const dispatch = createEventDispatcher();
    let container: HTMLElement;
    let editor: any;
    let isInternalChange = false;

    // 配置项
    const options = {
        theme: 'tailwind', // 支持 Tailwind!
        iconlib: 'fontawesome5', // 或者 null
        disable_edit_json: true,
        disable_properties: true,
        disable_collapse: true,
        no_additional_properties: true,
        schema: schema,
        startval: value
    };

    onMount(() => {
        if (container && schema) {
            editor = new JSONEditor(container, options);

            // 监听表单变化
            editor.on('change', () => {
                isInternalChange = true;
                const val = editor.getValue();
                dispatch('change', val);
                // 也可以直接 bind:value
                value = val;
                isInternalChange = false;
            });
        }
    });

    // 监听外部 value 变化（双向绑定）
    $: if (editor && value && !isInternalChange) {
        // 只有当外部真正改变了值（比如切了工具）才重置编辑器
        // 注意：频繁 setValue 会导致光标重置，所以这里需要防抖或 diff
        // 简单起见，假设外部 value 只有初始化时会变
        if (JSON.stringify(value) !== JSON.stringify(editor.getValue())) {
             editor.setValue(value);
        }
    }

    // 监听 schema 变化（切换工具）
    $: if (editor && schema) {
        // 这是一个重操作，通常需要销毁重建
        // 为了简单，我们在父组件用 {#key} 来重建组件
    }

    onDestroy(() => {
        if (editor) editor.destroy();
    });
</script>

<div bind:this={container} class="schema-form-container"></div>
<style>
    /* 容器基础样式 */
    :global(.schema-form-container) {
        font-size: 0.8rem; /* 整体字体变小 */
    }

    /* 隐藏它自带的丑陋标题卡片背景 */
    :global(.schema-form-container .card) {
        border: none !important;
        box-shadow: none !important;
        background: transparent !important;
        padding: 0 !important;
    }

    /* 针对 Object 的标题 */
    :global(.schema-form-container h3) {
        font-size: 0.75rem !important;
        font-weight: 700 !important;
        text-transform: uppercase;
        letter-spacing: 0.05em;
        color: currentColor;
        opacity: 0.7;
        margin-bottom: 0.25rem;
        margin-top: 1rem;
    }

    /* 针对字段 Label */
    :global(.schema-form-container label) {
        font-size: 0.75rem !important;
        font-weight: 600 !important;
    }

    /* 【核心修改】说明文字 (Description) */
    :global(.schema-form-container p),
    :global(.schema-form-container .form-text) {
        font-size: 0.7rem !important; /* 极小字体 */
        color: currentColor !important;
        opacity: 0.5 !important;       /* 降低透明度 */
        margin-top: 0.1rem !important;
        margin-bottom: 0.5rem !important;
        line-height: 1.2 !important;
        font-style: italic;            /* 斜体，表示这是注释 */
    }

    /* 输入框紧凑化 */
    :global(.schema-form-container input),
    :global(.schema-form-container select) {
        padding: 0.25rem 0.5rem !important;
        height: auto !important;
        min-height: 1.75rem !important;
        font-size: 0.8rem !important;
    }
</style>
