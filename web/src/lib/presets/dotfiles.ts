import type { SetupEntry } from '../api/generated/index.js';
import { ownJson, put, type Change } from './change.js';

const DOTFILES = {
	kind: 'file',
	target: '.config/corral-agents/dotfiles.json',
	secret: false
} as const;

/**
 * The repository checked out into the home at the first start: no `url` for none, no `branch`
 * for its default one.
 */
export type Dotfiles = { url: string; branch: string };

export function readDotfiles(entries: SetupEntry[]): Dotfiles {
	const { url, branch } = ownJson(entries, DOTFILES);
	return {
		url: typeof url === 'string' ? url : '',
		branch: typeof branch === 'string' ? branch : ''
	};
}

export function writeDotfiles(current: SetupEntry[], form: Dotfiles): Change[] {
	const url = form.url.trim();
	const branch = form.branch.trim() || null;
	return put(current, DOTFILES, url ? `${JSON.stringify({ url, branch }, null, 2)}\n` : null);
}

/** `host/path, branch` without the scheme, a login or `.git`, or `null` when there is none. */
export function dotfilesSummary({ url, branch }: Dotfiles): string | null {
	if (!url) {
		return null;
	}
	const shown = url.replace(/^[a-z][a-z0-9+.-]*:\/\/([^/@]*@)?/i, '').replace(/\.git\/?$/, '');
	return branch ? `${shown}, ${branch}` : shown;
}
