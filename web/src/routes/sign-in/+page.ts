import { redirect } from '@sveltejs/kit';
import { providers, signedInUser } from '#lib/api/auth.js';
import { returnPath } from '#lib/sign-in.js';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ url }) => {
	const next = returnPath(url.searchParams.get('next'));
	const [user, listed] = await Promise.all([signedInUser(), providers()]);
	if (user) {
		redirect(307, next);
	}
	return { providers: listed, next, error: url.searchParams.get('error') };
};
