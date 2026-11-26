import { api } from './client.js';
import type { AllowanceChange, OwnedSandbox, SandboxDetails, User } from './generated/index.js';

export function users(): Promise<User[]> {
	return api.get<User[]>('/api/admin/users');
}

export function setEnabled(user: User, enabled: boolean): Promise<User> {
	return api.post<User>(`/api/admin/users/${user.id}/${enabled ? 'enable' : 'disable'}`);
}

/** How many sandboxes the user may have in place of the limit, or with `null` the limit again. */
export function setAllowance(user: User, maxSandboxes: number | null): Promise<User> {
	return api.put<User>(`/api/admin/users/${user.id}/allowance`, {
		maxSandboxes
	} satisfies AllowanceChange);
}

/** Deletes the user with their sandboxes, keys and environment, or finishes a failed deletion. */
export function deleteUser(user: User): Promise<void> {
	return api.delete(`/api/admin/users/${user.id}`);
}

export function allSandboxes(): Promise<OwnedSandbox[]> {
	return api.get<OwnedSandbox[]>('/api/admin/sandboxes');
}

/** The sandbox with what its engine reports about how it runs. */
export function sandboxDetails(handle: string, name: string): Promise<SandboxDetails> {
	return api.get<SandboxDetails>(`/api/admin/sandboxes/${handle}/${name}`);
}
