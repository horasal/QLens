import { writable } from 'svelte/store';
import type { ChatEntry, ChatMeta, Toast } from '../types';
import type { CompletionUsage } from '../types';

// --- 基础 UI 状态 ---
export const isLoading = writable<boolean>(false);
export const isDragging = writable<boolean>(false);
export const connectionStatus = writable<'connected' | 'disconnected' | 'reconnecting'>('disconnected');

// --- 聊天数据 ---
export const historyList = writable<ChatMeta[]>([]);
export const currentChat = writable<ChatEntry | null>(null);
export const currentUsage = writable<CompletionUsage | null>(null);

// --- 正在处理中的 Chat IDs (Set 的 Store 封装) ---
function createSetStore<T>() {
    const { subscribe, update, set } = writable<Set<T>>(new Set());
    return {
        subscribe,
        add: (item: T) => update((s) => {
            const n = new Set(s);
            n.add(item);
            return n;
        }),
        delete: (item: T) => update((s) => {
            const n = new Set(s);
            n.delete(item);
            return n;
        }),
        clear: () => set(new Set()),
        has: (item: T) => {
            let hasItem = false;
            update(s => { hasItem = s.has(item); return s; });
            return hasItem;
        }
    };
}
export const processingChatIds = createSetStore<string>();

// 用于后端流式传输时，如果当前不在该聊天窗口，先存起来
export const wipDeltaStore = writable<Map<string, any[]>>(new Map());

// --- Toast 通知系统 (全局) ---
function createToastStore() {
    const { subscribe, update } = writable<Toast[]>([]);
    let counter = 0;

    return {
        subscribe,
        show: (message: string, type: 'error' | 'info' | 'success' = 'error', duration = 3000) => {
            const id = counter++;
            update(t => [...t, { id, message, type }]);
            setTimeout(() => {
                update(t => t.filter(item => item.id !== id));
            }, duration);
        },
        dismiss: (id: number) => {
            update(t => t.filter(item => item.id !== id));
        }
    };
}
export const toasts = createToastStore();
