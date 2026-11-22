import { goto } from '$app/navigation';
import { signInPath } from '#lib/sign-in.js';
import type { ErrorBody } from './generated/index.js';

/** A failed call, with the API's `ErrorBody` message. Status 0 when Corral could not be reached. */
export class ApiError extends Error {
	readonly status: number;

	constructor(status: number, body: ErrorBody) {
		super(body.error);
		this.status = status;
	}
}

type Method = 'GET' | 'POST' | 'PUT' | 'DELETE';

type Options = {
	body?: unknown;
	/** Whether a 401 sends the browser to the sign-in page, which then returns here. */
	redirectSignedOut?: boolean;
};

export async function request<T>(
	method: Method,
	path: string,
	{ body, redirectSignedOut = true }: Options = {}
): Promise<T> {
	let response: Response;
	try {
		response = await fetch(path, {
			method,
			// The API wants this on every POST and PUT, bodiless ones included.
			headers: method === 'POST' || method === 'PUT' ? { 'Content-Type': 'application/json' } : {},
			body: body === undefined ? undefined : JSON.stringify(body)
		});
	} catch {
		throw new ApiError(0, { error: 'Corral cannot be reached' });
	}
	const text = await response.text();
	if (response.ok) {
		return (text ? JSON.parse(text) : undefined) as T;
	}
	const error = new ApiError(response.status, errorBody(text, response));
	if (response.status === 401 && redirectSignedOut) {
		await goto(signInPath(location.pathname + location.search), { replace: true });
	}
	throw error;
}

function errorBody(text: string, response: Response): ErrorBody {
	try {
		const body: unknown = JSON.parse(text);
		if (typeof body === 'object' && body !== null && 'error' in body) {
			return { error: String(body.error) };
		}
	} catch {
		// Not the API's JSON, such as a proxy's error page.
	}
	return { error: `${response.status} ${response.statusText}`.trim() };
}

export const api = {
	get: <T>(path: string, options?: Options) => request<T>('GET', path, options),
	post: <T = void>(path: string, body?: unknown, options?: Options) =>
		request<T>('POST', path, { ...options, body }),
	put: <T = void>(path: string, body?: unknown, options?: Options) =>
		request<T>('PUT', path, { ...options, body }),
	delete: <T = void>(path: string, options?: Options) => request<T>('DELETE', path, options)
};
