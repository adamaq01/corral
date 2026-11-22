/** One letter, or two for a handle in parts such as `ada-lovelace`. */
export function initials(handle: string): string {
	const [first = '', second] = handle.split('-').filter(Boolean);
	return (second ? first[0] + second[0] : first.slice(0, 1)).toUpperCase();
}

/** A colour of its own for each handle, the same on every visit and for every viewer. */
export function handleColour(handle: string): string {
	let hash = 0;
	for (const char of handle) {
		hash = (Math.imul(hash, 31) + char.charCodeAt(0)) >>> 0;
	}
	return `oklch(0.55 0.15 ${hash % 360})`;
}
