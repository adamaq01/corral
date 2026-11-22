import { ApiError, api } from './client.js';
import type { Me, Provider } from './generated/index.js';

export function providers(): Promise<Provider[]> {
	return api.get<Provider[]>('/auth/providers');
}

/** The signed-in user, or `null` when nobody is. */
export async function signedInUser(): Promise<Me | null> {
	try {
		return await api.get<Me>('/api/me', { redirectSignedOut: false });
	} catch (error) {
		if (error instanceof ApiError && error.status === 401) {
			return null;
		}
		throw error;
	}
}
