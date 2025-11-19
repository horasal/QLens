<script lang="ts">
	type ToastType = 'error' | 'info' | 'success';
	type Toast = {
		id: number;
		message: string;
		type: ToastType;
	};

	let toasts: Toast[] = [];
	let toastCounter = 0;

	// 替代旧的 showErrorToast
	export function showToast(message: string, type: ToastType = 'error', duration = 3000) {
		const id = toastCounter++;
		const newToast: Toast = { id, message, type };

		// 赋值触发 Svelte 更新
		toasts = [...toasts, newToast];

		setTimeout(() => {
			dismissToast(id);
		}, duration);
	}

	function dismissToast(id: number) {
		toasts = toasts.filter((t) => t.id !== id);
	}
</script>

<div class="toast toast-center toast-top z-50 flex flex-col gap-2">
	{#each toasts as toast (toast.id)}
		<div
			class="alert shadow-lg transition-all duration-300"
			class:alert-error={toast.type === 'error'}
			class:alert-info={toast.type === 'info'}
			class:alert-success={toast.type === 'success'}
		>
			<span>{toast.message}</span>
			<button class="btn btn-circle btn-ghost btn-xs" on:click={() => dismissToast(toast.id)}
				>✕</button
			>
		</div>
	{/each}
</div>
