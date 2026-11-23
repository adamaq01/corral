import type { EnvironmentEntry, StoredValue, ValueInput } from '#lib/api/generated/index.js';

const bytes = new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 });

/** A size in bytes, as people read it. */
export function size(count: number): string {
	if (count < 1024) {
		return `${count} ${count === 1 ? 'byte' : 'bytes'}`;
	}
	if (count < 1024 * 1024) {
		return `${bytes.format(count / 1024)} KiB`;
	}
	return `${bytes.format(count / 1024 / 1024)} MiB`;
}

/** One line saying what a value is without showing a secret one, such as `ghp_…Xz, 40 characters`. */
export function summary(value: StoredValue): string {
	switch (value.shown) {
		case 'text':
			return value.text;
		case 'binary':
			return `Binary, ${size(value.size)}`;
		case 'masked': {
			const length = `${value.length} ${value.length === 1 ? 'character' : 'characters'}`;
			return value.head === null ? length : `${value.head}…${value.tail}, ${length}`;
		}
		case 'hidden':
			return size(value.size);
	}
}

/** How the entry lands in a sandbox: `NAME` or `~/path`. */
export function where(entry: EnvironmentEntry): string {
	return entry.kind === 'variable' ? entry.target : `~/${entry.target}`;
}

/** The value a file input holds, as the API takes it. */
export async function fileValue(file: File): Promise<ValueInput> {
	const data = new Uint8Array(await file.arrayBuffer());
	let binary = '';
	for (let at = 0; at < data.length; at += 0x8000) {
		binary += String.fromCharCode(...data.subarray(at, at + 0x8000));
	}
	return { base64: btoa(binary) };
}
