import { api } from './client.js';
import type { LocalAccount, NewAccount, NewPassword, PasswordChange } from './generated/index.js';

/** Changes the password of the signed-in local account, which `current` must match. */
export function changePassword(current: string, password: string): Promise<void> {
	return api.put('/auth/local/password', { current, password } satisfies PasswordChange);
}

/** The accounts a provider keeps, managed at the `accounts` URL it lists. */
export function accounts(url: string) {
	const path = (username: string) => `${url}/${encodeURIComponent(username)}`;
	return {
		list: () => api.get<LocalAccount[]>(url),
		create: (account: NewAccount) => api.post(url, account),
		setPassword: (username: string, password: string) =>
			api.put(`${path(username)}/password`, { password } satisfies NewPassword),
		/** Deletes the account with the user who signs in with it and everything they own. */
		remove: (username: string) => api.delete(path(username))
	};
}
