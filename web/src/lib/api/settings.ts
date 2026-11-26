import { api } from './client.js';
import type {
	CatalogChange,
	CatalogSettings,
	LimitsChange,
	LimitsSettings,
	RegistriesSettings,
	RegistryChange
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
