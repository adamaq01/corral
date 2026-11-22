import { api } from './client.js';
import type { Credential, CredentialValue } from './generated/index.js';

const path = (name: string) => `/api/credentials/${encodeURIComponent(name)}`;

export const credentials = {
	list: () => api.get<Credential[]>('/api/credentials'),
	set: (name: string, value: CredentialValue) => api.put(path(name), value),
	remove: (name: string) => api.delete(path(name))
};
