import { writable, get } from 'svelte/store';

export type AppSettings = {
    model: string;
    temperature: number;
    maxTokens: number;
    topP: number;
    customSystemPrompt: string;
    enterToSend: boolean; // Enter 发送还是 Shift+Enter 发送
    parallelFunctionCall: boolean;
    systemPromptLang: string;
};

const defaultSettings: AppSettings = {
    model: 'Qwen3-VL-30B-A3B',
    temperature: 0.8,
    maxTokens: 20000,
    topP: 0.8,
    customSystemPrompt: "",
    enterToSend: true,
    parallelFunctionCall: false,
    systemPromptLang: 'en',
};

function createSettingsStore() {
    const store = writable<AppSettings>(defaultSettings);
    const { subscribe, set, update } = store;

    if (typeof localStorage !== 'undefined') {
        const saved = localStorage.getItem('app_settings');
        if (saved) {
            try {
                const parsed = JSON.parse(saved);
                set({ ...defaultSettings, ...parsed });
            } catch (e) {
                console.error('Failed to load settings', e);
            }
        }
    }

    return {
        subscribe,
        set: (val: AppSettings) => {
            if (typeof localStorage !== 'undefined') {
                localStorage.setItem('app_settings', JSON.stringify(val));
            }
            set(val);
        },
        update: (fn: (val: AppSettings) => AppSettings) => {
            update(n => {
                const newVal = fn(n);
                if (typeof localStorage !== 'undefined') {
                    localStorage.setItem('app_settings', JSON.stringify(newVal));
                }
                return newVal;
            });
        },
        getLLMConfig: () => {
          const val = get(store);
          return {
            model: val.model,
            temp: val.temperature,
            custom_system_prompt: val.customSystemPrompt,
            max_completion_tokens: val.maxTokens,
            top_p: val.topP,
            parallel_function_call: val.parallelFunctionCall,
            system_prompt_language: val.systemPromptLang === '' ? null:val.systemPromptLang,
          };
        },
        reset: () => set(defaultSettings)
    };
}

export const settings = createSettingsStore();
export const showSettings = writable(false);
export const showArtifacts = writable(false);
