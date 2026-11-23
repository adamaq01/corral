import { api } from './client.js';
import type {
	AttachedEntry,
	EntryChange,
	EnvironmentEntry,
	NewEntry,
	ValueInput
} from './generated/index.js';

const entry = (id: string) => `/api/environment/${encodeURIComponent(id)}`;
const attachment = (sandbox: string, id: string) =>
	`/api/sandboxes/${encodeURIComponent(sandbox)}/environment/${encodeURIComponent(id)}`;

export const environment = {
	list: () => api.get<EnvironmentEntry[]>('/api/environment'),
	get: (id: string) => api.get<EnvironmentEntry>(entry(id)),
	create: (request: NewEntry) => api.post<EnvironmentEntry>('/api/environment', request),
	change: (id: string, change: EntryChange) => api.put<EnvironmentEntry>(entry(id), change),
	/** Detaches the entry from every sandbox, which loses it at its next start. */
	remove: (id: string) => api.delete(entry(id)),
	ofSandbox: (sandbox: string) =>
		api.get<AttachedEntry[]>(`/api/sandboxes/${encodeURIComponent(sandbox)}/environment`),
	attach: (sandbox: string, id: string) => api.put(attachment(sandbox, id)),
	detach: (sandbox: string, id: string) => api.delete(attachment(sandbox, id)),
	override: (sandbox: string, id: string, value: ValueInput) =>
		api.put(`${attachment(sandbox, id)}/override`, value),
	clearOverride: (sandbox: string, id: string) => api.delete(`${attachment(sandbox, id)}/override`)
};
