import { api } from './client.js';
import type {
	AttachedEntry,
	EntryChange,
	SetupEntry,
	NewEntry,
	TemplatePreview,
	ValueInput
} from './generated/index.js';

const entry = (id: string) => `/api/setup/${encodeURIComponent(id)}`;
const attachment = (sandbox: string, id: string) =>
	`/api/sandboxes/${encodeURIComponent(sandbox)}/setup/${encodeURIComponent(id)}`;

export const setup = {
	list: () => api.get<SetupEntry[]>('/api/setup'),
	get: (id: string) => api.get<SetupEntry>(entry(id)),
	create: (request: NewEntry) => api.post<SetupEntry>('/api/setup', request),
	change: (id: string, change: EntryChange) => api.put<SetupEntry>(entry(id), change),
	/** Detaches the entry from every sandbox, which loses it at its next start. */
	remove: (id: string) => api.delete(entry(id)),
	ofSandbox: (sandbox: string) =>
		api.get<AttachedEntry[]>(`/api/sandboxes/${encodeURIComponent(sandbox)}/setup`),
	attach: (sandbox: string, id: string) => api.put(attachment(sandbox, id)),
	detach: (sandbox: string, id: string) => api.delete(attachment(sandbox, id)),
	override: (sandbox: string, id: string, value: ValueInput) =>
		api.put(`${attachment(sandbox, id)}/override`, value),
	clearOverride: (sandbox: string, id: string) => api.delete(`${attachment(sandbox, id)}/override`),
	/** The template as the sandbox would get it at its next start, secret variables masked. */
	preview: (sandbox: string, id: string) =>
		api.get<TemplatePreview>(`${attachment(sandbox, id)}/preview`)
};
