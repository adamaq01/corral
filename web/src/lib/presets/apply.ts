import { setup } from '#lib/api/setup.js';
import type { Change } from './change.js';

/**
 * Saves a preset's changes through the Setup API, one at a time: the removals first, so a new
 * entry never meets the one it replaces, then the changes, then the creates.
 */
export async function applyChanges(changes: Change[]): Promise<void> {
	for (const change of changes) {
		if ('remove' in change) {
			await setup.remove(change.remove);
		}
	}
	for (const change of changes) {
		if ('change' in change) {
			const { id, settings, value } = change.change;
			await setup.change(id, { ...settings, value });
		}
	}
	for (const change of changes) {
		if ('create' in change) {
			await setup.create(change.create);
		}
	}
}
