import { api } from './client.js';
import type { LimitsSettings } from './generated/index.js';

export function limits(): Promise<LimitsSettings> {
	return api.get<LimitsSettings>('/api/admin/settings/limits');
}
