/** Copies `text`, also over plain HTTP, where browsers withhold the Clipboard API. */
export async function copyText(text: string): Promise<void> {
	if (window.isSecureContext && navigator.clipboard) {
		await navigator.clipboard.writeText(text);
		return;
	}
	const area = document.createElement('textarea');
	area.value = text;
	area.setAttribute('readonly', '');
	area.style.position = 'fixed';
	area.style.opacity = '0';
	document.body.append(area);
	area.select();
	try {
		if (!document.execCommand('copy')) {
			throw new Error('the browser refused to copy');
		}
	} finally {
		area.remove();
	}
}
