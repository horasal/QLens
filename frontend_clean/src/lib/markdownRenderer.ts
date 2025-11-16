import markdownit from 'markdown-it';
import markdownItKatex from '@vscode/markdown-it-katex';
import DOMPurify from 'dompurify';
import hljs from 'highlight.js';

// 1. 初始化 markdown-it
const md = markdownit({
  html: false,         // 允许 KaTeX 生成的 HTML
  linkify: true,      // 自动转换 URL 为链接
  highlight: (str, lang) => {
          if (lang && hljs.getLanguage(lang)) {
              try {
                  return (
                      '<pre class="hljs"><code>' +
                      hljs.highlight(str, { language: lang, ignoreIllegals: true }).value +
                      '</code></pre>'
                  );
              } catch (__) {
              }
          }
          // 默认转义
          return '<pre class="hljs"><code>' + md.utils.escapeHtml(str) + '</code></pre>';
      }
  });

md.use(markdownItKatex);

type RenderOptions = {
	disableImages?: boolean;
};

/**
 * 渲染包含 Markdown 和 LaTeX 的文本
 * @param text - 来自 LLM 的原始文本
 * @returns - 经过清理可以安全渲染的 HTML 字符串
 */
  const imageRegex = /!\[(.*?)\]\((.*?)\)/g;

  export function renderMarkdown(text: string, options: RenderOptions = {}): string {
  	let processedText = text;

  	if (options.disableImages) {
  		// 避免 <img> 渲染
      // qwen会在推理时产生奇怪的路径，可以避免大量404请求
  		processedText = processedText.replace(imageRegex, '`$&`');
  	}
  const rawHtml = md.render(processedText);

  const cleanHtml = DOMPurify.sanitize(rawHtml, {
		ADD_TAGS: [
			'math', 'mstyle', 'mspace', 'svg', 'path', 'g', 'span',
			'mfrac', 'mi', 'mn', 'mo', 'mover', 'msub', 'msup',
			'mtable', 'mtd', 'mtr', 'mrow', 'msqrt', 'munderover',
		],
		ADD_ATTR: [
			'style', // 允许 style 属性
			'viewbox', 'xmlns', 'd', 'width', 'height', 'aria-hidden', 'href'
		],
	});

  return cleanHtml;
}
