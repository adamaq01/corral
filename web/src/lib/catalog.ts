import type { Choice } from '#lib/api/generated/index.js';

/** A size in bytes as people read it: whole GiB where it is one, else MiB. */
export function bytesLabel(bytes: number): string {
	const gib = 1024 ** 3;
	return bytes % gib === 0 ? `${bytes / gib} GiB` : `${Math.ceil(bytes / 1024 ** 2)} MiB`;
}

/** A catalog entry's label, or its key while the catalog loads or once it no longer offers it. */
export function choiceLabel(choices: Choice[] | undefined, key: string): string {
	return choices?.find((choice) => choice.key === key)?.label ?? key;
}
