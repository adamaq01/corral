import { api } from './client.js';
import type { Catalog, CreateSandbox, Sandbox, Wake } from './generated/index.js';

const path = (name: string) => `/api/sandboxes/${encodeURIComponent(name)}`;

export const sandboxes = {
	list: () => api.get<Sandbox[]>('/api/sandboxes'),
	get: (name: string) => api.get<Sandbox>(path(name)),
	create: (request: CreateSandbox) => api.post<Sandbox>('/api/sandboxes', request),
	wake: (name: string) => api.post<Wake>(`${path(name)}/wake`),
	stop: (name: string) => api.post(`${path(name)}/stop`),
	restart: (name: string) => api.post(`${path(name)}/restart`),
	makeDefault: (name: string) => api.post(`${path(name)}/default`),
	/** Cancels a deletion `corral delete` scheduled; the sandbox stays stopped. */
	cancelDeletion: (name: string) => api.delete(`${path(name)}/deletion`),
	remove: (name: string) => api.delete(path(name)),
	catalog: () => api.get<Catalog>('/api/catalog')
};
