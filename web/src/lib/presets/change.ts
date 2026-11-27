import type {
	EntryChange,
	EntryKind,
	NewEntry,
	SetupEntry,
	ValueInput
} from '../api/generated/index.js';
import { settings } from '../setup.js';

/** One call to the Setup API a preset makes when it saves. */
export type Change =
	| { create: NewEntry }
	| { change: { id: string; settings: EntryChange; value?: ValueInput } }
	| { remove: string };

/** A name or path a preset writes, and whether what it holds there is secret. */
export type Slot = { kind: EntryKind; target: string; secret: boolean };

/** The member's own entry at `slot`'s name or path, which the preset reads and writes. */
export function own(entries: SetupEntry[], slot: Pick<Slot, 'kind' | 'target'>) {
	return entries.find(
		(entry) => entry.shared === null && entry.kind === slot.kind && entry.target === slot.target
	);
}

export function ownText(entries: SetupEntry[], slot: Pick<Slot, 'kind' | 'target'>) {
	const value = own(entries, slot)?.value;
	return value?.shown === 'text' ? value.text : null;
}

/** The JSON object of the member's own plain file at `slot`, `{}` when it holds none. */
export function ownJson(
	entries: SetupEntry[],
	slot: Pick<Slot, 'kind' | 'target'>
): Record<string, unknown> {
	try {
		const parsed: unknown = JSON.parse(ownText(entries, slot) ?? '{}');
		return isObject(parsed) ? parsed : {};
	} catch {
		return {};
	}
}

export function isObject(value: unknown): value is Record<string, unknown> {
	return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/**
 * The changes that leave `slot` holding `text`, or no entry for `null`: nothing when it already
 * does, else a create, a value replacement that keeps the entry's other settings, or a removal.
 */
export function put(current: SetupEntry[], slot: Slot, text: string | null): Change[] {
	const held = own(current, slot);
	if (text === null) {
		return held ? [{ remove: held.id }] : [];
	}
	if (!held) {
		const { kind, target, secret } = slot;
		return [{ create: { kind, target, secret, applyToNew: true, value: { text } } }];
	}
	if (held.secret === slot.secret && held.value.shown === 'text' && held.value.text === text) {
		return [];
	}
	return [
		{
			change: { id: held.id, settings: { ...settings(held), secret: slot.secret }, value: { text } }
		}
	];
}
