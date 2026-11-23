import { api } from './client.js';
import type { SandboxDefaults } from './generated/index.js';

/** Deletes the signed-in user with everything they own, and ends the session. */
export function deleteAccount(): Promise<void> {
	return api.delete('/api/me');
}

/** The image and size the signed-in user's new sandboxes get unless they pick others. */
export function setSandboxDefaults(defaults: SandboxDefaults): Promise<void> {
	return api.put('/api/me/sandbox-defaults', defaults);
}
