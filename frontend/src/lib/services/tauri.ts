export async function tauriPathsToWebFiles(paths: string[]): Promise<File[]> {
    const { readFile, stat } = await import('@tauri-apps/plugin-fs');
    const { basename } = await import('@tauri-apps/api/path');

    const files: File[] = [];
    for (const path of paths) {
        try {
            const contents = await readFile(path);

            const name = await basename(path);

            let lastModified = Date.now();
            try {
                const fstat = await stat(path);
                if (fstat.mtime) lastModified = fstat.mtime.getTime();
            } catch (err) {
                console.warn(`Failed to stat file ${path}: ${err}, using current time.`);
            }

            const mimeType = getMimeType(name);

            const file = new File([contents], name, {
                type: mimeType,
                lastModified: lastModified,
            });
            files.push(file);
        } catch (e) {
            console.error(`Failed to load tauri file: ${path}`, e);
        }
    }
    return files;
}

function getMimeType(filename: string): string {
	const ext = filename.split('.').pop()?.toLowerCase();
	switch (ext) {
		case 'png':
			return 'image/png';
		case 'jpg':
		case 'jpeg':
			return 'image/jpeg';
		case 'gif':
			return 'image/gif';
		case 'webp':
			return 'image/webp';
		case 'svg':
			return 'image/svg+xml';
		case 'txt':
			return 'text/plain';
		case 'md':
			return 'text/markdown';
		case 'json':
			return 'application/json';
		case 'js':
		case 'ts':
			return 'text/javascript';
		default:
			return 'application/octet-stream';
	}
}
