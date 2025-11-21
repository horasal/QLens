<script lang="ts">
	import { settings } from '$lib/stores/settingsStore';
	import { _ } from 'svelte-i18n'; // 引入 i18n
	import { onMount } from 'svelte';

	export let show = false;
	let availableModels: string[] = [];
    let isLoadingModels = false;

    onMount(async () => { await load_model_list() });

    async function load_model_list() {
      isLoadingModels = true;
      try {
          const res = await fetch('/api/models');
          if (res.ok) {
              availableModels = await res.json();
          }
      } catch (e) {
          console.error("Failed to fetch models", e);
      } finally {
          isLoadingModels = false;
      }
    }

    function close() { show = false; }
</script>

<dialog class="modal" class:modal-open={show}>
	<div class="modal-box flex max-h-[90vh] w-11/12 max-w-2xl flex-col overflow-hidden">
		<h3 class="mb-4 flex-shrink-0 text-lg font-bold">{$_('settings_title')}</h3>

		<div class="flex-1 overflow-y-auto px-1 py-2">
			<div class="flex flex-col gap-6">
				<div class="form-control">
					<label class="label">
						<span class="label-text font-bold">{$_('setting_system_prompt')}</span>
					</label>
					<textarea
						class="textarea-bordered textarea h-24 w-full font-mono text-sm"
						placeholder={$_('setting_system_prompt_placeholder')}
						bind:value={$settings.customSystemPrompt}
					></textarea>
					<label class="label">
						<span class="label-text-alt whitespace-normal text-base-content/60">
							{$_('setting_system_prompt_hint')}
						</span>
					</label>
				</div>
                    <label class="label">
                        <span class="label-text font-bold">System Prompt Language</span>
                    </label>
                    <select class="select select-bordered w-full" bind:value={$settings.systemPromptLang}>
                        <option value="auto">Auto Detect</option>
                        <option value="en">English (Default)</option>
                        <option value="zh">Chinese</option>
                        <option value="ja">Japanese</option>
                        <option value="ko">Korean</option>
                    </select>
                    <label class="label">
                        <span class="label-text-alt text-base-content/60">
                            Force the AI to use a specific language for tool usage and self-awareness.
                        </span>
                    </label>
                </div>

                <div class="form-control">
                    <label class="label">
                        <span class="label-text font-bold">{$_('setting_model_name')}</span>
                        {#if isLoadingModels}
                            <span class="loading loading-spinner loading-xs"></span>
                        {/if}
                    </label>

                    <input
                        type="text"
                        list="model-options"
                        class="input input-bordered w-full"
                        placeholder="Select or type model name..."
                        bind:value={$settings.model}
                    />

                    <datalist id="model-options">
                        {#each availableModels as modelName}
                            <option value={modelName}></option>
                        {/each}
                    </datalist>

                    <label class="label">
                        <span class="label-text-alt text-base-content/60">
                            Type directly or select from the fetched list.
                        </span>
                    </label>
                </div>
				<div class="grid grid-cols-1 gap-4 md:grid-cols-2">
					<div class="form-control">
						<label class="label">
							<span class="label-text">{$_('setting_temperature')}: {$settings.temperature}</span>
						</label>
						<input
							type="range"
							min="0"
							max="2"
							step="0.1"
							class="range range-primary range-sm"
							bind:value={$settings.temperature}
						/>
					</div>
				</div>

				<div class="form-control">
					<label class="label cursor-pointer items-start justify-between">
						<div class="mr-4 flex flex-col gap-1">
							<span class="label-text font-bold">{$_('setting_parallel_tool')}</span>
							<span class="label-text-alt whitespace-normal text-base-content/60">
								{$_('setting_parallel_tool_hint')}
							</span>
						</div>
						<input
							type="checkbox"
							class="toggle flex-shrink-0 toggle-primary"
							bind:checked={$settings.parallelFunctionCall}
						/>
					</label>
				</div>
				<div class="form-control">
					<label class="label cursor-pointer items-center justify-between">
						<div class="flex flex-col">
							<span class="label-text font-bold">{$_('setting_enter_to_send')}</span>
							<span class="label-text-alt text-base-content/60">
								{$settings.enterToSend
									? 'Current: Enter to send, Shift+Enter to newline'
									: 'Current: Ctrl+Enter to send, Enter to newline'}
							</span>
						</div>
						<input
							type="checkbox"
							class="toggle toggle-primary"
							bind:checked={$settings.enterToSend}
						/>
					</label>
				</div>
			</div>

		<div class="modal-action flex-shrink-0">
			<button class="btn" on:click={close}>{$_('close')}</button>
		</div>
	</div>
	<form method="dialog" class="modal-backdrop">
		<button on:click={close}>close</button>
	</form>
</dialog>
