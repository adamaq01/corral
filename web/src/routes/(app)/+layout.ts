import { redirect } from '@sveltejs/kit';
import { providers, signedInUser } from '#lib/api/auth.js';
import { signInPath } from '#lib/sign-in.js';
import type { LayoutLoad } from './$types';

export const load: LayoutLoad = async ({ url, untrack }) => {
	const [user, listed] = await Promise.all([signedInUser(), providers()]);
	if (!user) {
		redirect(307, signInPath(untrack(() => url.pathname + url.search)));
	}
	return { user, providers: listed };
};
