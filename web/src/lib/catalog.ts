import type { Choice } from '#lib/api/generated/index.js';

/** A catalog entry's label, or its key while the catalog loads or once it no longer offers it. */
export function choiceLabel(choices: Choice[] | undefined, key: string): string {
	return choices?.find((choice) => choice.key === key)?.label ?? key;
}
