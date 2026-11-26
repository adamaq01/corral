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
	const left = before.by === 'token' ? before.host : '';
	const text = withKeys(ownText(current, GITCONFIG) ?? '', presetKeys(form, login, left));
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

/**
 * The keys the preset writes for `form`, each with its value or `null` to remove it: the login's
 * username goes from the host it had, `left`, when the form no longer logs in there.
 */
function presetKeys(form: GitForm, login: Login | null, left: string): ConfigKey[] {
	const empty = form.name === '' && form.email === '' && form.reach.by === 'nothing';
	const username = (host: string, value: string | null): ConfigKey => ({
		section: 'credential',
		subsection: `https://${host}`,
		key: 'username',
		value
	});
	return [
		{ section: 'user', subsection: null, key: 'name', value: form.name || null },
		{ section: 'user', subsection: null, key: 'email', value: form.email || null },
		{ section: 'credential', subsection: null, key: 'helper', value: login && 'store' },
		...(left && left !== login?.host ? [username(left, null)] : []),
		...(login?.host ? [username(login.host, login.user)] : []),
		{ section: 'init', subsection: null, key: 'defaultBranch', value: empty ? null : 'main' }
	];
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

function headerLine(section: string, subsection: string | null): string {
	return subsection === null
		? `[${section}]\n`
		: `[${section} "${subsection.replace(/["\\]/g, '\\$&')}"]\n`;
}

type Section = { section: string; subsection: string | null; head: string; body: string[] };

/**
 * A git config text's sections in order, each its header line and the lines under it. The lines
 * before the first header are a section named `''` with no header.
 */
function sections(text: string): Section[] {
	const found: Section[] = [{ section: '', subsection: null, head: '', body: [] }];
	for (const line of text.split(/(?<=\n)/)) {
		const opened = header(line);
		if (opened) {
			found.push({ ...opened, head: line, body: [] });
		} else {
			found[found.length - 1].body.push(line);
		}
	}
	return found;
}

/** The key of a `key = value` line, lowercased as git compares keys, and its value. */
function assignment(line: string): { key: string; value: string } | null {
	const found = /^([A-Za-z][A-Za-z0-9-]*)\s*=\s*(.*)$/.exec(line.trim());
	return found ? { key: found[1].toLowerCase(), value: unquoted(found[2]) } : null;
}

type ConfigLine = { section: string; subsection: string | null; key: string; value: string };
type ConfigKey = Omit<ConfigLine, 'value'> & { value: string | null };

/** The `key = value` lines of a git config text, with their section, enough to read it back. */
function parseGitConfig(text: string): ConfigLine[] {
	return sections(text).flatMap(({ section, subsection, body }) =>
		body.flatMap((line) => {
			const found = assignment(line);
			return found ? [{ section, subsection, ...found }] : [];
		})
	);
}

/**
 * `text` with each of `keys` set in place, or removed for a `null` value, and every other line as
 * it was. A key with no section gets one, and a section the removals leave blank goes.
 */
function withKeys(text: string, keys: ConfigKey[]): string {
	const config = sections(text === '' || text.endsWith('\n') ? text : `${text}\n`);
	const emptied = new Set<Section>();
	for (const { section, subsection, key, value } of keys) {
		let line = value === null ? null : `\t${key} = ${configValue(value)}\n`;
		const held = config.filter(
			(each) => each.section === section && each.subsection === subsection
		);
		for (const each of held) {
			each.body = each.body.flatMap((raw) => {
				if (assignment(raw)?.key !== key.toLowerCase()) {
					return [raw];
				}
				const replaced = line;
				line = null;
				if (replaced === null) {
					emptied.add(each);
				}
				return replaced === null ? [] : [replaced];
			});
		}
		if (line === null) {
			continue;
		}
		if (held.length > 0) {
			const { body } = held[0];
			body.splice(body.findLastIndex((raw) => raw.trim() !== '') + 1, 0, line);
		} else {
			config.splice(newSectionAt(config, section, subsection), 0, {
				section,
				subsection,
				head: headerLine(section, subsection),
				body: [line]
			});
		}
	}
	return config
		.filter((each) => !emptied.has(each) || each.body.some((raw) => raw.trim() !== ''))
		.map((each) => each.head + each.body.join(''))
		.join('');
}

/**
 * Where a new section goes: a login's before the other logins', since `readGit` takes the first
 * login as the preset's, and any other at the end.
 */
function newSectionAt(config: Section[], section: string, subsection: string | null): number {
	const logins = config.findIndex(
		(each) => each.section === 'credential' && each.subsection !== null
	);
	return section === 'credential' && subsection !== null && logins !== -1 ? logins : config.length;
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
