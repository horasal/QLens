import markdownit from 'markdown-it';
import markdownItKatex from '@vscode/markdown-it-katex';
import markdownItLinkAttributes from 'markdown-it-link-attributes';
import DOMPurify from 'dompurify';
import hljs from 'highlight.js';
import { getApiBase } from './services/baseUrl';

const md = markdownit({
	html: false,
	linkify: true,
	breaks: true
});

md.use(markdownItKatex);

// 链接安全配置
md.use(markdownItLinkAttributes, {
	attrs: {
		target: '_blank',
		rel: 'noopener noreferrer',
		class: 'link link-primary hover:underline'
	}
});

md.renderer.rules.fence = function (tokens, idx, options, env, self) {
	const token = tokens[idx];
	const info = token.info ? md.utils.unescapeAll(token.info).trim() : '';
	const [lang] = info.split(/\s+/);
	const code = token.content;

	let highlightedCode = '';

	// 高亮逻辑
	if (lang && hljs.getLanguage(lang)) {
		try {
			highlightedCode = hljs.highlight(code, { language: lang, ignoreIllegals: true }).value;
		} catch (__) {}
	}
	// 兜底转义
	if (!highlightedCode) {
		highlightedCode = md.utils.escapeHtml(code);
	}

	// 生成最终 HTML (macOS 风格代码卡片)
	return `
    <div class="code-card my-4 overflow-hidden rounded-xl border border-base-300 bg-[#282c34] shadow-sm text-left group/code">

      <div class="flex h-9 items-center justify-between border-b border-white/10 bg-[#21252b] px-3 select-none">
        <div class="flex items-center gap-2">
          <div class="flex gap-1.5">
            <div class="h-2.5 w-2.5 rounded-full bg-[#ff5f56]"></div>
            <div class="h-2.5 w-2.5 rounded-full bg-[#ffbd2e]"></div>
            <div class="h-2.5 w-2.5 rounded-full bg-[#27c93f]"></div>
          </div>
          <span class="ml-2 font-mono text-[10px] font-bold uppercase tracking-wider text-gray-500">${lang || 'TEXT'}</span>
        </div>

        <button class="copy-code-btn flex items-center justify-center rounded-md p-1.5 text-gray-400 transition-all hover:bg-white/10 hover:text-white active:scale-95 opacity-0 group-hover/code:opacity-100" title="Copy Code">
          <span class="icon-copy pointer-events-none flex items-center">
              <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/></svg>
          </span>
        </button>
      </div>

      <div class="relative">
          <pre class="!bg-transparent !m-0 !p-4 overflow-x-auto text-sm leading-relaxed scrollbar-thin scrollbar-thumb-gray-600 scrollbar-track-transparent"><code class="font-mono !bg-transparent text-[#abb2bf]">${highlightedCode}</code></pre>
      </div>
    </div>
  `;
};

const defaultImageRender =
	md.renderer.rules.image ||
	function (tokens, idx, options, env, self) {
		return self.renderToken(tokens, idx, options);
	};

md.renderer.rules.image = function (tokens, idx, options, env, self) {
	const token = tokens[idx];
	let src = token.attrGet('src') || '';
	const alt = token.content;

	const apiBase = getApiBase();
	console.log(apiBase);
	if (src.startsWith('/api') && apiBase) {
		src = `${apiBase}${src}`;
		token.attrSet('src', src);
	}

	if (env && env.disableImages) {
		return `
      <a
        href="${md.utils.escapeHtml(src)}"
        target="_blank"
        rel="noopener noreferrer"
        class="inline-flex items-center gap-1 px-2 py-1 rounded bg-base-200 text-xs text-base-content/60 border border-base-300 select-none transition-colors hover:bg-base-300 hover:text-primary !no-underline"
        title="Click to open image source"
      >
        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor" class="w-3 h-3">
            <path fill-rule="evenodd" d="M1 5.25A2.25 2.25 0 013.25 3h13.5A2.25 2.25 0 0119 5.25v9.5A2.25 2.25 0 0116.75 17H3.25A2.25 2.25 0 011 14.75v-9.5zm1.5 5.81v3.69c0 .414.336.75.75.75h13.5a.75.75 0 00.75-.75v-2.69l-2.22-2.219a.75.75 0 00-1.06 0l-1.91 1.909.47.47a.75.75 0 11-1.06 1.06L6.53 8.091a.75.75 0 00-1.06 0l-2.97 2.97zM12 7a1 1 0 11-2 0 1 1 0 012 0z" clip-rule="evenodd" /></svg>
        Image: ${md.utils.escapeHtml(alt || 'Untitled')}
        <span class="opacity-50 ml-1">↗</span>
      </a>
    `;
	}

	return defaultImageRender(tokens, idx, options, env, self);
};

type RenderOptions = {
	disableImages?: boolean;
};

export function renderMarkdown(text: string, options: RenderOptions = {}): string {
	const rawHtml = md.render(text, { disableImages: options.disableImages });

	const cleanHtml = DOMPurify.sanitize(rawHtml, {
		ADD_TAGS: [
			'math',
			'mstyle',
			'mspace',
			'mfrac',
			'mi',
			'mn',
			'mo',
			'mover',
			'msub',
			'msup',
			'mtable',
			'mtd',
			'mtr',
			'mrow',
			'msqrt',
			'munderover'
		],
		ADD_ATTR: ['style', 'target', 'rel', 'class']
	});

	return cleanHtml;
}
