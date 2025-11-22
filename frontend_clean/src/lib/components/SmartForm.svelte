<script lang="ts">
	import { createEventDispatcher, onMount } from 'svelte';

	// 当前节点的 schema
	export let schema: any;
	// 绑定的值
	export let value: any;
	// 路径 (用于 label 显示)
	export let path: string = '';

	// 根 Schema，用于查找 $ref
	export let rootSchema: any = schema;

	const dispatch = createEventDispatcher();

	// 如果当前 schema 是引用 {$ref: "#/$defs/Bbox"}，则找到真实定义
	function resolveSchema(s: any): any {
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
			return current;
		}
		return s;
	}

	// 计算出当前真正要用的 schema 定义
	$: effectiveSchema = resolveSchema(schema);

	function initValue(s: any) {
		s = resolveSchema(s); // 初始化时也要解析 ref
		if (!s) return null;

		if (s.type === 'string') return '';
		if (s.type === 'number' || s.type === 'integer') return 0;
		if (s.type === 'boolean') return false;
		if (s.type === 'array') return [];
		if (s.type === 'object') {
			// 如果是 Object，预先初始化 properties
			const obj: any = {};
			if (s.properties) {
				for (const key in s.properties) {
					obj[key] = initValue(s.properties[key]);
				}
			}
			return obj;
		}
		// 多类型情况 (type: ["string", "null"])
		if (Array.isArray(s.type)) {
			if (s.type.includes('string')) return '';
			if (s.type.includes('null')) return null;
		}
		return null;
	}

	onMount(() => {
		if (value === undefined && effectiveSchema) {
			value = initValue(effectiveSchema);
			notifyChange();
		}
	});

	function notifyChange() {
		dispatch('change', value);
	}

	function handleInput(e: Event) {
		const target = e.target as HTMLInputElement;
		// 处理数字转换
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
		const itemSchema = resolveSchema(effectiveSchema.items);
		value = [...value, initValue(itemSchema)];
		notifyChange();
	}

	function removeArrayItem(index: number) {
		if (!Array.isArray(value)) return;
		value = value.filter((_, i) => i !== index);
		notifyChange();
	}

	// 判断是否为紧凑数字数组 (bbox)
	function isCompactNumArray(s: any) {
		if (s.type !== 'array') return false;
		const items = resolveSchema(s.items);
		return items && (items.type === 'number' || items.type === 'integer');
	}

	// 判断类型是否包含 null (Option<T>)
	function isNullable(s: any) {
		return Array.isArray(s.type) && s.type.includes('null');
	}

	// 获取主要类型 (排除 null)
	function getPrimaryType(s: any) {
		if (Array.isArray(s.type)) {
			return s.type.find((t: string) => t !== 'null');
		}
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
						<span class="ml-2 font-normal italic opacity-50 text-[10px]">{effectiveSchema.description}</span>
					{/if}
				</label>

				{#if isNullable(effectiveSchema)}
					<label class="label cursor-pointer p-0">
						<span class="label-text mr-2 text-[10px] opacity-50">Enabled</span>
						<input
							type="checkbox"
							class="checkbox checkbox-xs checkbox-primary"
							checked={value !== null}
							on:change={(e) => {
								if (e.currentTarget.checked) {
									// 恢复默认值
									value = initValue({ ...effectiveSchema, type: getPrimaryType(effectiveSchema) });
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

		{#if value === null && isNullable(effectiveSchema)}
			<div class="text-xs italic opacity-30 py-1 pl-2 border-l-2 border-base-200">Null</div>
		{:else}
			{@const primaryType = getPrimaryType(effectiveSchema)}

			{#if primaryType === 'string'}
				{#if effectiveSchema.enum}
					<select class="select select-bordered select-sm w-full text-xs" bind:value on:change={notifyChange}>
						{#each effectiveSchema.enum as opt}
							<option value={opt}>{opt}</option>
						{/each}
					</select>
				{:else}
					<input
						type="text"
						class="input input-bordered input-sm w-full text-xs font-mono"
						bind:value
						on:input={handleInput}
						placeholder={effectiveSchema.description || ''}
					/>
				{/if}

			{:else if primaryType === 'number' || primaryType === 'integer'}
				<input
					type="number"
					class="input input-bordered input-sm w-full text-xs font-mono"
					value={value}
					on:input={handleInput}
				/>

			{:else if primaryType === 'boolean'}
				<input type="checkbox" class="toggle toggle-primary toggle-sm" bind:checked={value} on:change={notifyChange} />

			{:else if primaryType === 'object' && effectiveSchema.properties}
				<div class="rounded-lg border border-base-200 bg-base-50/50 p-3">
					{#each Object.keys(effectiveSchema.properties) as key}
						<svelte:self
							schema={effectiveSchema.properties[key]}
							value={value ? value[key] : undefined}
							path={key}
							rootSchema={rootSchema}
							on:change={(e) => handleObjectPropChange(key, e.detail)}
						/>
					{/each}
				</div>

			{:else if primaryType === 'array'}
				{#if isCompactNumArray(effectiveSchema)}
					<div class="flex gap-2 rounded-lg border border-base-200 bg-base-50 p-2">
						{#each value || [] as item, i}
							<input
								type="number"
								class="input input-bordered input-xs w-full min-w-0 text-center font-mono text-xs"
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
						{#if (!value || value.length === 0)}
							<button class="btn btn-xs btn-ghost text-[10px]" on:click={() => { value = [0,0,0,0]; notifyChange(); }}>Set [0,0,0,0]</button>
						{/if}
					</div>
				{:else}
					<div class="space-y-2 border-l-2 border-base-200 pl-2">
						{#each value || [] as item, i}
							<div class="relative group/arr-item">
								<svelte:self
									schema={effectiveSchema.items}
									value={item}
									path={`Item ${i + 1}`}
									rootSchema={rootSchema}
									on:change={(e) => handleArrayItemChange(i, e.detail)}
								/>
								<button
									class="btn btn-circle btn-xs btn-ghost absolute right-0 top-0 opacity-0 group-hover/arr-item:opacity-100 text-error"
									on:click={() => removeArrayItem(i)}
									title="Remove"
								>×</button>
							</div>
						{/each}
						<button class="btn btn-xs btn-ghost w-full border-dashed border-base-300" on:click={addArrayItem}>+ Add Item</button>
					</div>
				{/if}
			{/if}
		{/if}
	</div>
{/if}
