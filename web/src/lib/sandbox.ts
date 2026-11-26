/** The repository URLs the server takes: no login in an https:// one, no password in any. */
export const REPOSITORY_PATTERN =
	/^(?=[^\p{White_Space}\p{Cc}]{1,1024}$)(?:(?:https:\/\/|ssh:\/\/(?:(?!-)[^\/:]*@)?)(?!-)[^\/@]+(?:\/.*)?|file:\/\/\/.+|(?!.*:\/\/)(?!-)[^\/:@]+@(?!-)[^\/:@]+:.+)$/u;

export const REPOSITORY_RULE =
	'An https://, ssh:// or git@host:path address, with no password in it.';

/** The branch names the server takes. */
export const BRANCH_PATTERN = /^(?!-)(?!.*\.\.)[^\p{White_Space}\p{Cc}~^:?*\[\\]{1,255}$/u;

export const BRANCH_RULE =
	'A git branch name, with no spaces, .., ~, ^, :, ?, *, [ or \\ in it, and no - first.';

/** The directory in the home the server clones `url` into. */
export function cloneDirectory(url: string): string {
	const scheme = url.indexOf('://');
	const rest = scheme === -1 ? url : url.slice(scheme + 3);
	const split = rest.indexOf(scheme === -1 ? ':' : '/');
	const path = split === -1 ? '' : rest.slice(split + 1);
	const last = path.replace(/\/+$/, '').split('/').pop() ?? '';
	const name = last.endsWith('.git') ? last.slice(0, -4) : last;
	return ['', '.', '..'].includes(name) ? 'repo' : name;
}
