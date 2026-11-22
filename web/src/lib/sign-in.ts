const SIGN_IN = '/sign-in';

/** A path on this origin to come back to, as the server's own `next` rule allows, else `/`. */
export function returnPath(next: string | null): string {
	if (
		next === null ||
		!next.startsWith('/') ||
		next.startsWith('//') ||
		next.includes('\\') ||
		next === SIGN_IN ||
		next.startsWith(`${SIGN_IN}?`)
	) {
		return '/';
	}
	return next;
}

/** The sign-in page, showing `error` if given, which returns to `next` once signed in. */
export function signInPath(next: string, error?: string): string {
	const back = returnPath(next);
	const query = new URLSearchParams({
		...(back === '/' ? {} : { next: back }),
		...(error === undefined ? {} : { error })
	});
	return query.size === 0 ? SIGN_IN : `${SIGN_IN}?${query}`;
}
