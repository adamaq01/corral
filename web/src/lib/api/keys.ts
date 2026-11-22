import { api } from './client.js';
import type { NewKey, SshKey } from './generated/index.js';

export const keys = {
	list: () => api.get<SshKey[]>('/api/keys'),
	add: (key: NewKey) => api.post<SshKey>('/api/keys', key),
	remove: (id: string) => api.delete(`/api/keys/${encodeURIComponent(id)}`)
};
