import { writable } from 'svelte/store';

function createThemeStore() {
    const { subscribe, set, update } = writable(false);
    // false = Light(lofi), true = Dark(dim)

    return {
        subscribe,
        init: () => {
            if (typeof localStorage !== 'undefined') {
                const saved = localStorage.getItem('theme');
                const isDark = saved === 'dim';
                set(isDark);
                document.documentElement.setAttribute('data-theme', isDark ? 'dim' : 'lofi');
            }
        },
        toggle: () => {
            update(isDark => {
                const newIsDark = !isDark;
                const theme = newIsDark ? 'dim' : 'lofi';
                document.documentElement.setAttribute('data-theme', theme);
                localStorage.setItem('theme', theme);
                return newIsDark;
            });
        }
    };
}

export const themeStore = createThemeStore();
