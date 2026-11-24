import type { HandleClientError } from '@sveltejs/kit/hooks';
import { failureMessage } from '#lib/api/client.js';

export const handleError: HandleClientError = ({ kind, error }) =>
	kind === 'unknown' ? { message: failureMessage(error) } : undefined;
