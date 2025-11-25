import { register, init, getLocaleFromNavigator } from 'svelte-i18n';

register('en', () => import('../langs/en.json'));
register('zh', () => import('../langs/zh.json'));
register('zh-CN', () => import('../langs/zh.json'));
register('ko', () => import('../langs/ko.json'));
register('ja', () => import('../langs/ja.json'));

export function initI18n() {
	init({
		fallbackLocale: 'en',
		initialLocale: getLocaleFromNavigator()
	});
}
