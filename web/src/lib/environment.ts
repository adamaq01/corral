import type {
	EntryChange,
	EntryKind,
	EnvironmentEntry,
	StoredValue,
	ValueInput
} from '#lib/api/generated/index.js';
import { FILE_LIMIT, VARIABLE_LIMIT } from '#lib/api/generated/index.js';

const bytes = new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 });
const encoder = new TextEncoder();

/** A size in bytes, as people read it. */
export function size(count: number): string {
	if (count < 1024) {
		return `${count} ${count === 1 ? 'byte' : 'bytes'}`;
	}
	if (count < 1024 * 1024) {
		return `${bytes.format(count / 1024)} KiB`;
	}
	return `${bytes.format(count / 1024 / 1024)} MiB`;
}

/** `N characters`. */
export function characterCount(count: number): string {
	return `${count} ${count === 1 ? 'character' : 'characters'}`;
}

/** The lines in `text`, a newline ending the last one rather than starting another. */
export function lineCount(text: string): number {
	const lines = text.split('\n');
	return lines.at(-1) === '' ? lines.length - 1 : lines.length;
}

/** One line saying what a value is without showing a secret one. */
export function summary(value: StoredValue): string {
	switch (value.shown) {
		case 'text':
			return value.text;
		case 'binary':
			return `Binary, ${size(value.size)}`;
		case 'masked':
			return `Secret, ${characterCount(value.length)}`;
		case 'hidden':
			return `Secret, ${size(value.size)}`;
	}
}

/** How the entry lands in a sandbox: `NAME` or `~/path`. */
export function where(entry: Pick<EnvironmentEntry, 'kind' | 'target'>): string {
	return entry.kind === 'variable' ? entry.target : `~/${entry.target}`;
}

/** `variable` or `file`. */
export function noun(kind: EntryKind): string {
	return kind === 'variable' ? 'variable' : 'file';
}

/** A change that leaves the entry as it is, to spread a change over. */
export function settings(entry: EnvironmentEntry): EntryChange {
	const { kind, target, secret, template, executable, applyToNew } = entry;
	return { kind, target, secret, template, executable, applyToNew };
}

/** The value a file input holds, as the API takes it. */
export async function fileValue(file: File): Promise<ValueInput> {
	const data = new Uint8Array(await file.arrayBuffer());
	let binary = '';
	for (let at = 0; at < data.length; at += 0x8000) {
		binary += String.fromCharCode(...data.subarray(at, at + 0x8000));
	}
	return { base64: btoa(binary) };
}

/** The bytes a plain stored value holds, to download. */
export function valueBytes(value: StoredValue): Uint8Array<ArrayBuffer> | null {
	switch (value.shown) {
		case 'text':
			return encoder.encode(value.text);
		case 'binary':
			return Uint8Array.from(atob(value.base64), (char) => char.charCodeAt(0));
		default:
			return null;
	}
}

/** Saves `data` as a file named `name` in the browser's downloads. */
export function download(name: string, data: Uint8Array<ArrayBuffer> | string) {
	const url = URL.createObjectURL(new Blob([data], { type: 'application/octet-stream' }));
	const link = document.createElement('a');
	link.href = url;
	link.download = name;
	link.click();
	setTimeout(() => URL.revokeObjectURL(url), 1000);
}

/** The last part of a path, as a file name to download. */
export function baseName(path: string): string {
	return path.split('/').pop() || path;
}

// The sandbox and its sessions set these; Corral refuses them in any case.
const RESERVED = [
	'HOME',
	'HOSTNAME',
	'LOGNAME',
	'MAIL',
	'OLDPWD',
	'PATH',
	'PWD',
	'SHELL',
	'SHLVL',
	'TERM',
	'USER',
	'_'
];

/** Why `name` cannot be a variable's name, or `null` when it can, as Corral checks it. */
export function nameProblem(name: string): string | null {
	if (name === '') {
		return null;
	}
	if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) {
		return 'Use letters, digits and underscores, and do not start with a digit.';
	}
	if (name.length > 64) {
		return 'A name has at most 64 characters.';
	}
	const upper = name.toUpperCase();
	if (upper.startsWith('CORRAL_') || RESERVED.includes(upper)) {
		return `${name} is reserved: the sandbox sets it.`;
	}
	return null;
}

const HOME_PREFIXES = ['~/', '$HOME/', '${HOME}/', '/home/sandbox/', './'];

/** The path relative to the home, without the prefixes that name the home, as Corral keeps it. */
export function homePath(typed: string): string {
	let path = typed;
	for (let prefix = match(path); prefix; prefix = match(path)) {
		path = path.slice(prefix.length);
	}
	return path;

	function match(text: string) {
		return HOME_PREFIXES.find((prefix) => text.startsWith(prefix));
	}
}

/** Why `path` cannot be a file's path in the home, or `null` when it can. */
export function pathProblem(typed: string): string | null {
	const path = homePath(typed);
	if (typed === '') {
		return null;
	}
	if (
		path === '' ||
		path.startsWith('/') ||
		path.split('/').some((part) => part === '' || part === '.' || part === '..') ||
		path.trim() !== path
	) {
		return 'Give a path inside the home, such as .config/tool/config.toml.';
	}
	if (path === '.config' || path === '.config/corral' || path.startsWith('.config/corral/')) {
		return 'Corral keeps the sandbox’s own files in ~/.config/corral.';
	}
	return null;
}

/** Why a variable's value is too large, or `null`. */
export function variableTooLarge(name: string, text: string): string | null {
	const taken = encoder.encode(text).length + encoder.encode(name).length + 2;
	return taken > VARIABLE_LIMIT
		? `A variable takes at most ${size(VARIABLE_LIMIT)} with its name. Make it a file instead.`
		: null;
}

/** Why a file's content is too large, or `null`. */
export function fileTooLarge(count: number): string | null {
	return count > FILE_LIMIT
		? `This is ${size(count)}, more than the ${size(FILE_LIMIT)} a file may hold.`
		: null;
}

/** Bytes in `text` as UTF-8. */
export function utf8Size(text: string): number {
	return encoder.encode(text).length;
}

/** A variable read from a `.env` text. */
export type Assignment = { line: number; name: string; value: string };

/**
 * The variables of a `.env` text: `NAME=value` lines, with an optional `export`, quotes
 * around the value removed, and comments and blank lines skipped. Lines that are none of these
 * come back as problems.
 */
export function parseDotenv(text: string): {
	assignments: Assignment[];
	problems: { line: number; text: string }[];
} {
	const assignments: Assignment[] = [];
	const problems: { line: number; text: string }[] = [];
	text.split(/\r?\n/).forEach((raw, index) => {
		const line = raw.trim();
		if (line === '' || line.startsWith('#')) {
			return;
		}
		const found = /^(?:export\s+)?([^=\s]+)\s*=\s*(.*)$/.exec(line);
		if (!found) {
			problems.push({ line: index + 1, text: raw });
			return;
		}
		let value = found[2];
		const quote = value[0];
		if ((quote === '"' || quote === "'") && value.length >= 2 && value.endsWith(quote)) {
			value = value.slice(1, -1);
			if (quote === '"') {
				value = value.replace(/\\n/g, '\n').replace(/\\(["\\])/g, '$1');
			}
		} else {
			value = value.replace(/\s+#.*$/, '');
		}
		assignments.push({ line: index + 1, name: found[1], value });
	});
	return { assignments, problems };
}

/** A `.env` text of the plain variables, with a comment naming each secret one left out. */
export function toDotenv(entries: EnvironmentEntry[]): string {
	const lines = entries
		.filter((entry) => entry.kind === 'variable')
		.map((entry) =>
			entry.value.shown === 'text'
				? `${entry.target}=${quoted(entry.value.text)}`
				: `# ${entry.target} is secret; Corral never shows it again`
		);
	return `${lines.join('\n')}\n`;
}

function quoted(text: string): string {
	return /^[A-Za-z0-9_./:@,+-]*$/.test(text)
		? text
		: `"${text.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`;
}
