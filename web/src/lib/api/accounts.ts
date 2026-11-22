import { api } from './client.js';
import type { LocalAccount, NewAccount, NewPassword } from './generated/index.js';

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
