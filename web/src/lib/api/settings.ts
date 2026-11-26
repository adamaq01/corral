import { api } from './client.js';
import type {
	CatalogChange,
	CatalogSettings,
	LimitsChange,
	LimitsSettings,
	NewSharedEntry,
	RegistriesSettings,
	RegistryChange,
	SharedEntry,
	SharedEntryChange
} from './generated/index.js';

export function catalog(): Promise<CatalogSettings> {
	return api.get<CatalogSettings>('/api/admin/settings/catalog');
}

/** Replaces the part of the catalog the database keeps. */
export function setCatalog(change: CatalogChange): Promise<void> {
	return api.put('/api/admin/settings/catalog', change);
}

export function limits(): Promise<LimitsSettings> {
	return api.get<LimitsSettings>('/api/admin/settings/limits');
}

export function setLimits(change: LimitsChange): Promise<void> {
	return api.put('/api/admin/settings/limits', change);
}

export function registries(): Promise<RegistriesSettings> {
	return api.get<RegistriesSettings>('/api/admin/settings/registries');
}

/** Replaces the registries the database keeps. */
export function setRegistries(registries: RegistryChange[]): Promise<void> {
	return api.put('/api/admin/settings/registries', registries);
}

const sharedEntry = (id: string) => `/api/admin/settings/shared-setup/${encodeURIComponent(id)}`;

export function sharedSetup(): Promise<SharedEntry[]> {
	return api.get<SharedEntry[]>('/api/admin/settings/shared-setup');
}

export function createSharedEntry(entry: NewSharedEntry): Promise<SharedEntry> {
	return api.post<SharedEntry>('/api/admin/settings/shared-setup', entry);
}

export function changeSharedEntry(id: string, change: SharedEntryChange): Promise<SharedEntry> {
	return api.put<SharedEntry>(sharedEntry(id), change);
}

/** Deletes the entry with its value and every override; sandboxes lose it at their next start. */
export function removeSharedEntry(id: string): Promise<void> {
	return api.delete(sharedEntry(id));
}
