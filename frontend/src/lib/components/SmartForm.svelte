<script lang="ts">
	import { createEventDispatcher, onMount } from 'svelte';

	// 当前节点的 schema
	export let schema: any;
	// 绑定的值
	export let value: any;
	export let path: string = '';
	export let rootSchema: any = schema;

	const dispatch = createEventDispatcher();

	function resolveRef(s: any): any {
		if (s && s.$ref) {
			const refPath = s.$ref.replace('#/', '').split('/');
			let current = rootSchema;
			for (const p of refPath) {
				if (current && current[p]) {
					current = current[p];
				} else {
					console.warn('Schema ref not found:', s.$ref);
					return { type: 'string', description: 'Ref Error' };
				}
			}
			// 递归解析：因为 ref 指向的定义可能还是一个 ref (虽少见但可能)
			return resolveRef(current);
		}
		return s;
	}

	function normalizeSchema(s: any): any {
		// 先解开顶层引用
		let resolved = resolveRef(s);

		if (!resolved) return resolved;

		// 处理 Option<Enum> 产生的 anyOf: [{ $ref: ... }, { type: "null" }]
		if (resolved.anyOf || resolved.oneOf) {
			const variants = resolved.anyOf || resolved.oneOf;

			const meaningfulVariant = variants.find((v: any) => {
				const r = resolveRef(v);
				return r.type !== 'null';
			});

			if (meaningfulVariant) {
				// 递归解析那个分支 (因为它可能是一个 $ref 指向 Enum 定义)
				const inner = resolveRef(meaningfulVariant);

				// 返回合并后的 Schema：保留原始的 description，但使用内部的类型定义
				return {
					...inner,
					description: resolved.description || inner.description,
					// 标记一下，方便后续 isNullable 判断 (可选)
					__wasOptional: true
				};
			}
		}

		return resolved;
	}

	// 计算出用于渲染输入框的最终 Schema
	$: effectiveSchema = normalizeSchema(schema);

	function checkNullable(original: any, normalized: any): boolean {
		const s = resolveRef(original); // 看原始定义

		// 情况 A: type: ["string", "null"]
		if (Array.isArray(s.type) && s.type.includes('null')) return true;

		// 情况 B: anyOf: [..., {type: "null"}]
		if (s.anyOf || s.oneOf) {
			const variants = s.anyOf || s.oneOf;
			return variants.some((v: any) => {
				const r = resolveRef(v); // ref 可能是 null 定义
				return r.type === 'null';
			});
		}

		return false;
	}

	$: isNullable = checkNullable(schema, effectiveSchema);

	function initValue(s: any) {
		const norm = normalizeSchema(s);
		if (!norm) return null;

		let type = norm.type;
		if (Array.isArray(type)) {
			type = type.find((t: string) => t !== 'null');
		}

		if (type === 'string') return '';
		if (type === 'number' || type === 'integer') return 0;
		if (type === 'boolean') return false;
		if (type === 'array') return [];
		if (type === 'object') {
			const obj: any = {};
			if (norm.properties) {
				for (const key in norm.properties) {
					obj[key] = initValue(norm.properties[key]);
				}
			}
			return obj;
		}
		// 处理 Enum
		if (norm.enum && norm.enum.length > 0) {
			return norm.enum[0];
		}

		return null;
	}

	onMount(() => {
		// 如果值未定义，且不是 nullable (或者用户希望默认选中)，则初始化
		// 这里策略：如果是 nullable，默认给 null；如果不是，给默认值
		if (value === undefined) {
			if (isNullable) {
				value = null;
			} else if (effectiveSchema) {
				value = initValue(schema);
			}
			notifyChange();
		}
	});

	function notifyChange() {
		dispatch('change', value);
	}

	function handleInput(e: Event) {
		const target = e.target as HTMLInputElement;
		if (effectiveSchema.type === 'number' || effectiveSchema.type === 'integer') {
			value = target.value === '' ? 0 : Number(target.value);
		} else if (effectiveSchema.type === 'boolean') {
			value = target.checked;
		} else {
			value = target.value;
		}
		notifyChange();
	}

	function handleObjectPropChange(key: string, detail: any) {
		if (!value || typeof value !== 'object') value = {};
		value[key] = detail;
		notifyChange();
	}

	function handleArrayItemChange(index: number, detail: any) {
		if (!Array.isArray(value)) value = [];
		value[index] = detail;
		notifyChange();
	}

	function addArrayItem() {
		if (!Array.isArray(value)) value = [];
		const itemSchema = effectiveSchema.items; // normalized 后的 items 也是可以直接用的
		value = [...value, initValue(itemSchema)];
		notifyChange();
	}

	function removeArrayItem(index: number) {
		if (!Array.isArray(value)) return;
		value = value.filter((_, i) => i !== index);
		notifyChange();
	}

	function isCompactNumArray(s: any) {
		if (s.type !== 'array') return false;
		const items = normalizeSchema(s.items); // 记得 normalize items
		return items && (items.type === 'number' || items.type === 'integer');
	}

	// 获取用于显示的类型字符串
	function getPrimaryType(s: any) {
		if (Array.isArray(s.type)) return s.type.find((t: string) => t !== 'null');
		return s.type;
	}
</script>

{#if effectiveSchema}
	<div class="smart-form-field mb-3">
		{#if path}
			<div class="mb-1 flex items-baseline justify-between">
				<label class="block text-xs font-bold opacity-70">
					{path.split('.').pop()}
					{#if effectiveSchema.description}
						<span class="ml-2 text-[10px] font-normal italic opacity-50"
							>{effectiveSchema.description}</span
						>
					{/if}
				</label>

				{#if isNullable}
					<label class="label cursor-pointer p-0">
						<span class="label-text mr-2 text-[10px] opacity-50">Enabled</span>
						<input
							type="checkbox"
							class="checkbox checkbox-xs checkbox-primary"
							checked={value !== null}
							on:change={(e) => {
								if (e.currentTarget.checked) {
									const def = initValue(schema);
									if (def === null && getPrimaryType(effectiveSchema) === 'string') {
										value = '';
									} else {
										value = def;
									}
								} else {
									value = null;
								}
								notifyChange();
							}}
						/>
					</label>
				{/if}
			</div>
		{/if}

		{#if value === null && isNullable}
			<div class="border-l-2 border-base-200 py-1 pl-2 text-xs italic opacity-30">Null</div>
		{:else}
			{@const type = getPrimaryType(effectiveSchema)}

			{#if type === 'string'}
				{#if effectiveSchema.enum}
					<select
						class="select-bordered select w-full select-sm text-xs"
						bind:value
						on:change={notifyChange}
					>
						{#each effectiveSchema.enum as opt}
							<option value={opt}>{opt}</option>
						{/each}
					</select>
				{:else}
					<input
						type="text"
						class="input-bordered input input-sm w-full font-mono text-xs"
						bind:value
						on:input={handleInput}
						placeholder={effectiveSchema.description || ''}
					/>
				{/if}
			{:else if type === 'number' || type === 'integer'}
				<input
					type="number"
					class="input-bordered input input-sm w-full font-mono text-xs"
					{value}
					on:input={handleInput}
				/>
			{:else if type === 'boolean'}
				<input
					type="checkbox"
					class="toggle toggle-primary toggle-sm"
					bind:checked={value}
					on:change={notifyChange}
				/>
			{:else if type === 'object' && effectiveSchema.properties}
				<div class="bg-base-50/50 rounded-lg border border-base-200 p-3">
					{#each Object.keys(effectiveSchema.properties) as key}
						<svelte:self
							schema={effectiveSchema.properties[key]}
							value={value ? value[key] : undefined}
							path={key}
							{rootSchema}
							on:change={(e) => handleObjectPropChange(key, e.detail)}
						/>
					{/each}
				</div>
			{:else if type === 'array'}
				{#if isCompactNumArray(effectiveSchema)}
					<div class="bg-base-50 flex gap-2 rounded-lg border border-base-200 p-2">
						{#each value || [] as item, i}
							<input
								type="number"
								class="input-bordered input input-xs w-full min-w-0 text-center font-mono text-xs"
								value={item}
								on:input={(e) => {
									const val = Number(e.currentTarget.value);
									const newArr = [...value];
									newArr[i] = val;
									value = newArr;
									notifyChange();
								}}
							/>
						{/each}
						{#if !value || value.length === 0}
							<button
								class="btn text-[10px] btn-ghost btn-xs"
								on:click={() => {
									value = [0, 0, 0, 0];
									notifyChange();
								}}>Set [0,0,0,0]</button
							>
						{/if}
					</div>
				{:else}
					<div class="space-y-2 border-l-2 border-base-200 pl-2">
						{#each value || [] as item, i}
							<div class="group/arr-item relative">
								<svelte:self
									schema={effectiveSchema.items}
									value={item}
									path={`Item ${i + 1}`}
									{rootSchema}
									on:change={(e) => handleArrayItemChange(i, e.detail)}
								/>
								<button
									class="btn absolute top-0 right-0 btn-circle text-error opacity-0 btn-ghost btn-xs group-hover/arr-item:opacity-100"
									on:click={() => removeArrayItem(i)}
									title="Remove">×</button
								>
							</div>
						{/each}
						<button
							class="btn w-full border-dashed border-base-300 btn-ghost btn-xs"
							on:click={addArrayItem}>+ Add Item</button
						>
					</div>
				{/if}
			{/if}
		{/if}
	</div>
{/if}

<style>
    /* 针对 Chrome, Safari, Edge, Opera */
    input[type=number]::-webkit-outer-spin-button,
    input[type=number]::-webkit-inner-spin-button {
        -webkit-appearance: none;
        margin: 0;
    }

    /* 针对 Firefox */
    input[type=number] {
        -moz-appearance: textfield;
    }
</style>
