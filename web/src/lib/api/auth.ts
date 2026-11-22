import { redirect } from '@sveltejs/kit';
import { signInPath } from '#lib/sign-in.js';
import { ApiError, api } from './client.js';
import type { Me, PasswordLogin, Provider } from './generated/index.js';

export function providers(): Promise<Provider[]> {
	return api.get<Provider[]>('/auth/providers');
}

/**
 * The signed-in user, or `null` when nobody is. A session whose account may no longer sign in,
 * such as a disabled one, is ended, and the browser goes to the sign-in page, which says why.
 */
export async function signedInUser(): Promise<Me | null> {
	try {
		return await api.get<Me>('/api/me', { redirectSignedOut: false });
	} catch (error) {
		if (error instanceof ApiError && error.status === 401) {
			return null;
		}
		if (error instanceof ApiError && error.status === 403) {
			await signOut();
			redirect(307, signInPath('/', error.message));
		}
		throw error;
	}
}

export function passwordLogin(provider: Provider, form: PasswordLogin): Promise<Me> {
	return api.post<Me>(provider.login, form, { redirectSignedOut: false });
}

/** Where a redirect provider's login starts, coming back to `next`. */
export function redirectLogin(provider: Provider, next: string): string {
	return next === '/' ? provider.login : `${provider.login}?${new URLSearchParams({ next })}`;
}

export function signOut(): Promise<void> {
	return api.post('/auth/logout');
}
