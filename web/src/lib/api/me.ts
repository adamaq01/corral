import { api } from './client.js';

/** Deletes the signed-in user with everything they own, and ends the session. */
export function deleteAccount(): Promise<void> {
	return api.delete('/api/me');
}
