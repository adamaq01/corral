import { expect, test } from '@playwright/test';
import type {
	EntryKind,
	SetupEntry,
	Shared,
	StoredValue,
	ValueInput
} from '../src/lib/api/generated/index.js';
// These run in Node, outside Vite, where `#lib/*.js` does not resolve: the presets' modules, and
// what they import at run time, import each other by relative paths.
import type { Change } from '../src/lib/presets/change.js';
import { readDotfiles, writeDotfiles } from '../src/lib/presets/dotfiles.js';
import { readGit, writeGit, type GitForm } from '../src/lib/presets/git.js';
import { needsKey, readLlm, writeLlm, type LlmForm } from '../src/lib/presets/llm.js';
import { readMcp, writeMcp, type McpServer } from '../src/lib/presets/mcp.js';

const setAt = '2026-11-26T09:00:00Z';

/** The member's own entry at `target`, holding `text`, or a secret value for `null`. */
function entry(
	kind: EntryKind,
	target: string,
	text: string | null,
	shared: Shared | null = null
): SetupEntry {
	return {
		id: `${target}-id`,
		kind,
		target,
		secret: text === null,
		template: false,
		executable: false,
		applyToNew: true,
		value: stored(kind, text),
		createdAt: setAt,
		sandboxes: [],
		references: [],
		duplicate: false,
		shared,
		shadows: false,
		shadowed: false
	};
}

function stored(kind: EntryKind, text: string | null): StoredValue {
	if (text !== null) {
		return { setAt, size: text.length, shown: 'text', text };
	}
	return kind === 'variable'
		? { setAt, size: 40, shown: 'masked', length: 40, head: null, tail: null }
		: { setAt, size: 40, shown: 'hidden' };
}

/** The setup after Corral saves `changes`, as the Setup API lists it. */
function saved(before: SetupEntry[], changes: Change[]): SetupEntry[] {
	const text = (value: ValueInput) => ('text' in value ? value.text : '');
	return changes.reduce((entries, change) => {
		if ('remove' in change) {
			return entries.filter((held) => held.id !== change.remove);
		}
		if ('change' in change) {
			const { id, settings, value } = change.change;
			return entries.map((held) =>
				held.id === id
					? {
							...held,
							...settings,
							value: value ? stored(held.kind, settings.secret ? null : text(value)) : held.value
						}
					: held
			);
		}
		const { kind, target, secret, value } = change.create;
		return [...entries, entry(kind, target, secret ? null : text(value))];
	}, before);
}

const variable = (target: string, text: string | null, shared: Shared | null = null) =>
	entry('variable', target, text, shared);
const file = (target: string, text: string | null) => entry('file', target, text);

const created = (kind: EntryKind, target: string, secret: boolean, text: string): Change => ({
	create: { kind, target, secret, applyToNew: true, value: { text } }
});

test.describe('LLM providers', () => {
	const untouched = { url: '', key: null };
	const form = (
		anthropic: LlmForm['rows']['anthropic'],
		openai: LlmForm['rows']['openai'] = untouched
	): LlmForm => ({ rows: { anthropic, openai } });

	test('a key without an address is written as the API key alone', () => {
		expect(writeLlm([], form({ url: '', key: 'sk-ant-1' }))).toEqual([
			created('variable', 'ANTHROPIC_API_KEY', true, 'sk-ant-1')
		]);
	});

	test('a key with an address is also the bearer token, and both read back', () => {
		const changes = writeLlm([], form({ url: 'https://llm.home.example', key: 'sk-ant-1' }));

		expect(changes).toEqual([
			created('variable', 'ANTHROPIC_BASE_URL', false, 'https://llm.home.example'),
			created('variable', 'ANTHROPIC_API_KEY', true, 'sk-ant-1'),
			created('variable', 'ANTHROPIC_AUTH_TOKEN', true, 'sk-ant-1')
		]);
		expect(readLlm(saved([], changes))).toEqual({
			rows: {
				anthropic: { url: 'https://llm.home.example', urlLocked: false, keySet: true },
				openai: { url: '', urlLocked: false, keySet: false }
			}
		});
	});

	test('an address added to a kept key writes the address, and asks for the key again', () => {
		const current = [variable('ANTHROPIC_API_KEY', null)];
		const added = form({ url: 'https://llm.home.example', key: null });

		expect(writeLlm(current, added)).toEqual([
			created('variable', 'ANTHROPIC_BASE_URL', false, 'https://llm.home.example')
		]);
		expect(needsKey(current, added)).toEqual({ anthropic: true, openai: false });
	});

	test("clearing a row's address removes it and the bearer token, and keeps the key", () => {
		const current = [
			variable('ANTHROPIC_BASE_URL', 'https://llm.home.example'),
			variable('ANTHROPIC_API_KEY', null),
			variable('ANTHROPIC_AUTH_TOKEN', null)
		];
		const cleared = form({ url: '', key: null });

		expect(writeLlm(current, cleared)).toEqual([
			{ remove: 'ANTHROPIC_BASE_URL-id' },
			{ remove: 'ANTHROPIC_AUTH_TOKEN-id' }
		]);
		expect(needsKey(current, cleared)).toEqual({ anthropic: false, openai: false });
	});

	test('clearing a row removes each of its entries, and nothing else', () => {
		const current = [
			variable('ANTHROPIC_BASE_URL', 'https://llm.home.example'),
			variable('ANTHROPIC_API_KEY', null),
			variable('ANTHROPIC_AUTH_TOKEN', null),
			variable('OPENAI_BASE_URL', 'https://llm.home.example/openai'),
			variable('OPENAI_API_KEY', null),
			variable('GH_TOKEN', null)
		];

		expect(
			writeLlm(
				current,
				form({ url: '', key: '' }, { url: 'https://llm.home.example/openai', key: null })
			)
		).toEqual([
			{ remove: 'ANTHROPIC_BASE_URL-id' },
			{ remove: 'ANTHROPIC_API_KEY-id' },
			{ remove: 'ANTHROPIC_AUTH_TOKEN-id' }
		]);
	});

	test("the admin's addresses are never written, and the keys are written for them", () => {
		const current = [
			variable('ANTHROPIC_BASE_URL', 'https://gateway.example.lan', 'required'),
			variable('OPENAI_BASE_URL', 'https://gateway.example.lan/openai', 'required')
		];

		expect(readLlm(current).rows).toEqual({
			anthropic: { url: 'https://gateway.example.lan', urlLocked: true, keySet: false },
			openai: { url: 'https://gateway.example.lan/openai', urlLocked: true, keySet: false }
		});
		expect(
			writeLlm(
				current,
				form({ url: '', key: 'sk-ant-1' }, { url: 'https://elsewhere.example', key: 'sk-1' })
			)
		).toEqual([
			created('variable', 'ANTHROPIC_API_KEY', true, 'sk-ant-1'),
			created('variable', 'ANTHROPIC_AUTH_TOKEN', true, 'sk-ant-1'),
			created('variable', 'OPENAI_API_KEY', true, 'sk-1')
		]);
	});
});

test.describe('git identity', () => {
	const gitconfig = (credential: string) =>
		`[user]\n\tname = Ada Lovelace\n\temail = ada@example.com\n${credential}[init]\n\tdefaultBranch = main\n`;

	test('a token writes the identity, the helper and the login, which read back', () => {
		const form: GitForm = {
			name: 'Ada Lovelace',
			email: 'ada@example.com',
			reach: { by: 'token', token: { host: 'github.com', user: 'ada', token: 'ghp_s3cret' } }
		};
		const changes = writeGit([], form);

		expect(changes).toEqual([
			created(
				'file',
				'.gitconfig',
				false,
				gitconfig(
					'[credential]\n\thelper = store\n[credential "https://github.com"]\n\tusername = ada\n'
				)
			),
			created('file', '.git-credentials', true, 'https://ada:ghp_s3cret@github.com\n')
		]);
		expect(readGit(saved([], changes))).toEqual({
			name: 'Ada Lovelace',
			email: 'ada@example.com',
			reach: { by: 'token', host: 'github.com', user: 'ada' }
		});
	});

	test('an SSH key takes the place of a token', () => {
		const current = [
			file(
				'.gitconfig',
				gitconfig(
					'[credential]\n\thelper = store\n[credential "https://github.com"]\n\tusername = ada\n'
				)
			),
			file('.git-credentials', null)
		];
		const form: GitForm = {
			name: 'Ada Lovelace',
			email: 'ada@example.com',
			reach: {
				by: 'ssh',
				key: {
					privateKey:
						'-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaA==\n-----END OPENSSH PRIVATE KEY-----',
					publicKey: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5 ada@laptop\n'
				}
			}
		};
		const changes = writeGit(current, form);

		expect(changes).toEqual([
			{
				change: {
					id: '.gitconfig-id',
					settings: {
						kind: 'file',
						target: '.gitconfig',
						secret: false,
						template: false,
						executable: false,
						applyToNew: true
					},
					value: { text: gitconfig('') }
				}
			},
			{ remove: '.git-credentials-id' },
			created(
				'file',
				'.ssh/id_ed25519',
				true,
				'-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaA==\n-----END OPENSSH PRIVATE KEY-----\n'
			),
			created('file', '.ssh/id_ed25519.pub', false, 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5 ada@laptop\n')
		]);
		expect(readGit(saved(current, changes))).toEqual({
			name: 'Ada Lovelace',
			email: 'ada@example.com',
			reach: { by: 'ssh' }
		});
	});
});

test.describe('MCP servers', () => {
	const servers: McpServer[] = [
		{ name: 'fetch', runs: 'command', command: 'uvx', args: ['mcp-server-fetch'], env: {} },
		{
			name: 'docs',
			runs: 'url',
			url: 'https://docs.example/mcp',
			headers: { Authorization: 'Bearer t0ken' }
		}
	];
	const text = `${JSON.stringify(
		{
			fetch: { command: 'uvx', args: ['mcp-server-fetch'], env: {} },
			docs: {
				type: 'http',
				url: 'https://docs.example/mcp',
				headers: { Authorization: 'Bearer t0ken' }
			}
		},
		null,
		2
	)}\n`;

	test('servers read back while plain, and turning Secret on makes the file secret', () => {
		const plain = writeMcp([], { secret: false, servers });
		expect(plain).toEqual([created('file', '.config/corral-agents/mcp.json', false, text)]);
		const current = saved([], plain);
		expect(readMcp(current)).toEqual({ secret: false, servers });

		const secret = writeMcp(current, { secret: true, servers });
		expect(secret).toEqual([
			{
				change: {
					id: '.config/corral-agents/mcp.json-id',
					settings: {
						kind: 'file',
						target: '.config/corral-agents/mcp.json',
						secret: true,
						template: false,
						executable: false,
						applyToNew: true
					},
					value: { text }
				}
			}
		]);
		expect(readMcp(saved(current, secret))).toEqual({ secret: true });
	});
});

test.describe('dotfiles', () => {
	test('the repository and its branch are written, read back, and removed', () => {
		const changes = writeDotfiles([], { url: 'https://github.com/ada/dotfiles.git', branch: '' });
		expect(changes).toEqual([
			created(
				'file',
				'.config/corral-agents/dotfiles.json',
				false,
				'{\n  "url": "https://github.com/ada/dotfiles.git",\n  "branch": null\n}\n'
			)
		]);
		const current = saved([], changes);
		expect(readDotfiles(current)).toEqual({
			url: 'https://github.com/ada/dotfiles.git',
			branch: ''
		});

		expect(writeDotfiles(current, { url: '', branch: '' })).toEqual([
			{ remove: '.config/corral-agents/dotfiles.json-id' }
		]);
	});
});
