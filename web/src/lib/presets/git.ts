import type { SetupEntry } from '../api/generated/index.js';
import { own, ownText, put, type Change } from './change.js';

const GITCONFIG = { kind: 'file', target: '.gitconfig', secret: false } as const;
const CREDENTIALS = { kind: 'file', target: '.git-credentials', secret: true } as const;
const PRIVATE_KEY = { kind: 'file', target: '.ssh/id_ed25519', secret: true } as const;
const PUBLIC_KEY = { kind: 'file', target: '.ssh/id_ed25519.pub', secret: false } as const;

export type Login = { host: string; user: string };

export type GitState = {
	name: string;
	email: string;
	reach: ({ by: 'token' } & Login) | { by: 'ssh' } | { by: 'nothing' };
};

/** What the form saves; a `null` token or key keeps the one set. */
export type GitForm = {
	name: string;
	email: string;
	reach:
		| { by: 'token'; token: (Login & { token: string }) | null }
		| { by: 'ssh'; key: { privateKey: string; publicKey: string } | null }
		| { by: 'nothing' };
};

export function readGit(entries: SetupEntry[]): GitState {
	const config = parseGitConfig(ownText(entries, GITCONFIG) ?? '');
	const value = (section: string, key: string) =>
		config.find((line) => line.section === section && line.key === key)?.value ?? '';
	const scoped = config.find(
		(line) => line.section === 'credential' && line.subsection !== null && line.key === 'username'
	);
	let reach: GitState['reach'] = { by: 'nothing' };
	if (own(entries, CREDENTIALS)) {
		reach = {
			by: 'token',
			host: scoped?.subsection?.replace(/^https:\/\//, '') ?? '',
			user: scoped?.value ?? ''
		};
	} else if (own(entries, PRIVATE_KEY)) {
		reach = { by: 'ssh' };
	}
	return { name: value('user', 'name'), email: value('user', 'email'), reach };
}

export function writeGit(current: SetupEntry[], form: GitForm): Change[] {
	const { reach } = form;
	const before = readGit(current).reach;
	const login =
		reach.by !== 'token' ? null : (reach.token ?? (before.by === 'token' ? before : null));
	const empty = form.name === '' && form.email === '' && reach.by === 'nothing';
	const hosts = [before.by === 'token' ? before.host : '', login?.host ?? ''].filter(Boolean);
	const others = otherSections(ownText(current, GITCONFIG) ?? '', hosts);
	const text = (empty ? '' : gitconfig(form, login)) + others;
	const changes = put(current, GITCONFIG, text.trim() === '' ? null : text);
	if (reach.by !== 'token' || reach.token) {
		const token = reach.by === 'token' ? reach.token : null;
		changes.push(...put(current, CREDENTIALS, token && credentialsLine(token)));
	}
	if (reach.by !== 'ssh' || reach.key) {
		const key = reach.by === 'ssh' ? reach.key : null;
		changes.push(
			...put(current, PRIVATE_KEY, key && lines(key.privateKey)),
			...put(current, PUBLIC_KEY, key?.publicKey ? lines(key.publicKey) : null)
		);
	}
	return changes;
}

/** Whether a refusal from Corral names the file the token is kept in. */
export function namesToken(message: string): boolean {
	return message.includes(`file ${CREDENTIALS.target}`);
}

/** One line naming who commits and how git reaches repositories, or `null` when nothing is set. */
export function gitSummary(state: GitState): string | null {
	const who = [state.name, state.email && `<${state.email}>`].filter(Boolean).join(' ');
	const said = [who, reachSaid(state.reach)].filter(Boolean).join(', ');
	return said || null;
}

function reachSaid(reach: GitState['reach']): string {
	switch (reach.by) {
		case 'token':
			return reach.host ? `token for ${reach.host}` : 'token';
		case 'ssh':
			return 'SSH key';
		case 'nothing':
			return '';
	}
}

function gitconfig(form: GitForm, login: Login | null): string {
	const user = [
		form.name && `\tname = ${configValue(form.name)}\n`,
		form.email && `\temail = ${configValue(form.email)}\n`
	].join('');
	return [
		user && `[user]\n${user}`,
		login && '[credential]\n\thelper = store\n',
		login?.host &&
			`[credential "https://${login.host}"]\n\tusername = ${configValue(login.user)}\n`,
		'[init]\n\tdefaultBranch = main\n'
	]
		.filter(Boolean)
		.join('');
}

/** The line git's `store` helper keeps for `login`, its parts percent-encoded as git reads them. */
function credentialsLine({ host, user, token }: Login & { token: string }): string {
	return `https://${encodeURIComponent(user)}:${encodeURIComponent(token)}@${host}\n`;
}

/** `text` ending with a newline, which OpenSSH needs at the end of a key file. */
function lines(text: string): string {
	return text.endsWith('\n') ? text : `${text}\n`;
}

/** A value as git config reads it back: quoted when it holds what git would take otherwise. */
function configValue(value: string): string {
	return /^\s|\s$|["\\#;]/.test(value)
		? `"${value.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`
		: value;
}

/** A section header, `[section]` or `[section "subsection"]`, with its name and subsection. */
const HEADER = /^\s*\[\s*([^\s\]"]+)(?:\s+"((?:[^"\\]|\\.)*)")?\s*\]/;

function header(line: string): { section: string; subsection: string | null } | null {
	const found = HEADER.exec(line);
	return found
		? { section: found[1].toLowerCase(), subsection: found[2]?.replace(/\\(.)/g, '$1') ?? null }
		: null;
}

/**
 * `text` without the sections the preset writes: `[user]`, `[init]`, `[credential]`, and
 * `[credential "https://<host>"]` for each of `hosts`.
 */
function otherSections(text: string, hosts: string[]): string {
	const owned = new Set(['user', 'init', 'credential']);
	const scoped = new Set(hosts.map((host) => `https://${host}`));
	let skipping = false;
	return text
		.split(/(?<=\n)/)
		.filter((line) => {
			const found = header(line);
			if (found) {
				skipping =
					owned.has(found.section) &&
					(found.subsection === null ||
						(found.section === 'credential' && scoped.has(found.subsection)));
			}
			return !skipping;
		})
		.join('');
}

type ConfigLine = { section: string; subsection: string | null; key: string; value: string };

/** The `key = value` lines of a git config text, with their section, enough to read it back. */
function parseGitConfig(text: string): ConfigLine[] {
	const found: ConfigLine[] = [];
	let section = '';
	let subsection: string | null = null;
	for (const raw of text.split('\n')) {
		const line = raw.trim();
		const opened = header(line);
		if (opened) {
			({ section, subsection } = opened);
			continue;
		}
		const assignment = /^([A-Za-z][A-Za-z0-9-]*)\s*=\s*(.*)$/.exec(line);
		if (assignment) {
			found.push({
				section,
				subsection,
				key: assignment[1].toLowerCase(),
				value: unquoted(assignment[2])
			});
		}
	}
	return found;
}

function unquoted(raw: string): string {
	let value = '';
	let quoted = false;
	for (let at = 0; at < raw.length; at++) {
		const char = raw[at];
		if (char === '\\' && at + 1 < raw.length) {
			const next = raw[++at];
			value += next === 'n' ? '\n' : next === 't' ? '\t' : next;
		} else if (char === '"') {
			quoted = !quoted;
		} else if (!quoted && (char === '#' || char === ';')) {
			break;
		} else {
			value += char;
		}
	}
	return value.trim();
}
