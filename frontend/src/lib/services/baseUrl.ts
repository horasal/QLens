const isTauri = !!window.__TAURI__;

const BACKEND_PORT = 3000;

/**
 * 获取 HTTP API 的基础路径
 */
export function getApiBase(): string {
    if (isTauri) {
        // Tauri 模式：必须指向本地回环接口的固定端口
        return `http://127.0.0.1:${BACKEND_PORT}`;
    }

    return '';
}

/**
 * 获取 WebSocket 地址
 */
export function getWsUrl(): string {
    if (isTauri) {
        return `ws://127.0.0.1:${BACKEND_PORT}/api/chat`;
    }

    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const host = window.location.host;
    return `${protocol}//${host}/api/chat`;
}
