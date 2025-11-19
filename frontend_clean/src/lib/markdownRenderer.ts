import markdownit from 'markdown-it';
import markdownItKatex from '@vscode/markdown-it-katex';
import markdownItLinkAttributes from 'markdown-it-link-attributes';
import DOMPurify from 'dompurify';
import hljs from 'highlight.js';

const md = markdownit({
	html: false,
	linkify: true,
	breaks: true,
	highlight: (str, lang) => {
		// 生成高亮代码
		let highlightedCode = '';
		if (lang && hljs.getLanguage(lang)) {
			try {
				highlightedCode = hljs.highlight(str, { language: lang, ignoreIllegals: true }).value;
			} catch (__) {}
		}
		if (!highlightedCode) {
			highlightedCode = md.utils.escapeHtml(str);
		}

		return `
        <div class="relative group/code my-4">
          <div class="absolute right-2 top-2 opacity-0 transition-opacity group-hover/code:opacity-100 z-10">
              <button class="btn btn-xs btn-square btn-ghost bg-base-100/80 hover:bg-base-100 copy-code-btn" title="Copy code">
                  <span class="icon-[lucide--copy] w-4 h-4 pointer-events-none"></span>
              </button>
          </div>
          <pre class="hljs p-4 rounded-lg text-sm overflow-x-auto bg-[#0d1117] text-[#c9d1d9]"><div class="flex justify-between items-center mb-1 text-xs text-gray-500 select-none"><span>${lang || 'text'}</span></div><code>${highlightedCode}</code></pre>
        </div>
      `;
	}
});

md.use(markdownItKatex);

// 安全链接：强制所有链接在新标签页打开，并添加 noopener
md.use(markdownItLinkAttributes, {
	attrs: {
		target: '_blank',
		rel: 'noopener noreferrer',
		class: 'link link-primary hover:underline' // 使用 DaisyUI 的 link 样式
	}
});

type RenderOptions = {
	disableImages?: boolean;
};

// 保存默认的渲染器
const defaultImageRender =
	md.renderer.rules.image ||
	function (tokens, idx, options, env, self) {
		return self.renderToken(tokens, idx, options);
	};

md.renderer.rules.image = function (tokens, idx, options, env, self) {
	// 获取当前 render 的上下文配置（需要在 render 调用时传递 env，或者利用闭包）
	const token = tokens[idx];
	const src = token.attrGet('src');
	const alt = token.content;

	if (env && env.disableImages) {
		// 渲染为一个漂亮的占位符，而不是丑陋的 code block
		return `
      <span class="inline-flex items-center gap-1 px-2 py-1 rounded bg-base-200 text-xs text-base-content/60 border border-base-300 cursor-not-allowed select-none" title="Image generation disabled">
        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor" class="w-3 h-3"><path fill-rule="evenodd" d="M1 5.25A2.25 2.25 0 013.25 3h13.5A2.25 2.25 0 0119 5.25v9.5A2.25 2.25 0 0116.75 17H3.25A2.25 2.25 0 011 14.75v-9.5zm1.5 5.81v3.69c0 .414.336.75.75.75h13.5a.75.75 0 00.75-.75v-2.69l-2.22-2.219a.75.75 0 00-1.06 0l-1.91 1.909.47.47a.75.75 0 11-1.06 1.06L6.53 8.091a.75.75 0 00-1.06 0l-2.97 2.97zM12 7a1 1 0 11-2 0 1 1 0 012 0z" clip-rule="evenodd" /></svg>
        Image: ${md.utils.escapeHtml(alt || 'Untitled')}
      </span>
    `;
	}

	return defaultImageRender(tokens, idx, options, env, self);
};

export function renderMarkdown(text: string, options: RenderOptions = {}): string {
	// 传入 options 到 env 参数中，以便 rule 里面读取
	const rawHtml = md.render(text, { disableImages: options.disableImages });

	const cleanHtml = DOMPurify.sanitize(rawHtml, {
		// 移除 SVG 相关标签，保留 MathML
		ADD_TAGS: [
			'math',
			'mstyle',
			'mspace', // 'svg', 'path', 'g'
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
		ADD_ATTR: [
			'style',
			'target',
			'rel',
			'class' // 允许我们刚才加的 class 和 target
		]
	});

	return cleanHtml;
}
