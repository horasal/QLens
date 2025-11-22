import { writable } from 'svelte/store';

export const playgroundState = writable<{
    toolName: string;
    args: string;
    autoRun?: boolean;
} | null>(null);
