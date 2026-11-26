import type { Locator, Page, Route } from '@playwright/test';
import type {
	AttachedEntry,
	NewEntry,
	Sandbox,
	SetupEntry
} from '../src/lib/api/generated/index.js';
import { expect, test, idle } from './fixtures';

test.use({ role: 'member' });

const setAt = '2026-11-20T17:45:00Z';

function entry(extra: Partial<SetupEntry>): SetupEntry {
	return {
		id: '3f1c2a4e-5b6d-4e7f-8a9b-0c1d2e3f4a5b',
		kind: 'variable',
		target: 'GH_TOKEN',
		secret: true,
		template: false,
		executable: false,
		applyToNew: true,
		value: { setAt, size: 40, shown: 'masked', length: 40, head: 'ghp_', tail: 'Xz' },
		createdAt: setAt,
		sandboxes: [],
		references: [],
		duplicate: false,
		shared: null,
		shadows: false,
		shadowed: false,
		...extra
	};
}

const main: Sandbox = {
	name: 'main',
	image: 'dev',
	size: 'small',
	isDefault: true,
	ssh: { username: 'main.bob', host: 'corral.example.lan', port: 48291 },
	createdAt: setAt,
	staleSetup: false,
	idle,
	deleteAt: null,
	repository: null,
	state: 'running'
};

const gateway = entry({
	id: '2a0b1c2d-3e4f-4a5b-8c6d-7e8f9a0b1c2d',
	target: 'ANTHROPIC_BASE_URL',
	secret: false,
	shared: 'required',
	value: { setAt, size: 23, shown: 'text', text: 'https://llm.example.lan' },
	sandboxes: [{ sandbox: 'main', overridden: false, unresolved: [] }]
});

const gatewayToken = entry({
	id: '3b1c2d3e-4f5a-4b6c-9d7e-8f9a0b1c2d3e',
	target: 'ANTHROPIC_AUTH_TOKEN',
	shared: 'required',
	value: { setAt, size: 23, shown: 'hidden' },
	sandboxes: [{ sandbox: 'main', overridden: false, unresolved: [] }]
});

const openai = entry({
	id: '4c2d3e4f-5a6b-4c7d-8e8f-9a0b1c2d3e4f',
	target: 'OPENAI_BASE_URL',
	secret: false,
	shared: 'offered',
	value: { setAt, size: 26, shown: 'text', text: 'https://openai.example.lan' }
});

type Call = { method: string; path: string; body: unknown };

/** Answers each call to `path` with `reply`, and records it. */
async function record(page: Page, path: string, reply: (route: Route) => unknown, calls: Call[]) {
	await page.route(`**${path}`, async (route) => {
		const request = route.request();
		calls.push({
			method: request.method(),
			path: new URL(request.url()).pathname,
			body: request.postData() ? request.postDataJSON() : null
		});
		await route.fulfill(reply(route) as Parameters<Route['fulfill']>[0]);
	});
}

test('a secret variable shows while typed, and is sent as typed', async ({ page }) => {
	const calls: Call[] = [];
	await page.route('**/api/setup', async (route) => {
		if (route.request().method() === 'GET') {
			return route.fulfill({ json: [] });
		}
		calls.push({ method: 'POST', path: '', body: route.request().postDataJSON() });
		return route.fulfill({ status: 201, json: entry({}) });
	});
	await page.route(`**/api/setup/${entry({}).id}`, (route) => route.fulfill({ json: entry({}) }));
	await page.route('**/api/sandboxes', (route) => route.fulfill({ json: [] }));
	await page.goto('/setup');
	await page.getByRole('button', { name: 'Add variable' }).first().click();
	const dialog = page.getByRole('dialog', { name: 'Add a variable' });
	const name = dialog.getByLabel('Name');
	const value = dialog.getByLabel('Value', { exact: true });

	await name.fill('1token');
	await expect(dialog).toContainText('do not start with a digit');
	await name.fill('path');
	await expect(dialog).toContainText('path is reserved');
	await name.fill('GH_TOKEN');
	await expect(dialog.getByRole('button', { name: 'Show the value' })).toHaveCount(0);
	await dialog.getByRole('switch', { name: 'Secret' }).click();
	await value.fill('ghp_typed');
	await expect(value).toHaveAttribute('type', 'password');
	await dialog.getByRole('button', { name: 'Show the value' }).click();
	await expect(value).toHaveAttribute('type', 'text');
	const hide = dialog.getByRole('button', { name: 'Hide the value' });
	await expect(hide).toHaveAttribute('aria-pressed', 'true');
	await value.press('End');
	await value.pressSequentially('-more');
	await expect(value).toHaveValue('ghp_typed-more');
	await dialog.getByRole('switch', { name: 'Apply to new sandboxes' }).click();
	await dialog.getByRole('button', { name: 'Save variable' }).click();

	await expect(page).toHaveURL(`/setup/${entry({}).id}`);
	await expect(page.getByRole('img', { name: 'Secret value, 40 characters' })).toContainText(
		'ghp_••••••••••••Xz'
	);
	expect(calls.map((call) => call.body)).toEqual([
		{
			kind: 'variable',
			target: 'GH_TOKEN',
			secret: true,
			template: false,
			executable: false,
			applyToNew: false,
			value: { text: 'ghp_typed-more' }
		}
	]);
});

test('a variable refuses pasted lines and a value past its limit', async ({ page }) => {
	await page.route('**/api/setup', (route) => route.fulfill({ json: [] }));
	await page.goto('/setup');
	await page.getByRole('button', { name: 'Add variable' }).first().click();
	const dialog = page.getByRole('dialog', { name: 'Add a variable' });
	await dialog.getByLabel('Name').fill('http_proxy');
	const value = dialog.getByLabel('Value', { exact: true });
	await value.focus();
	await page.evaluate(() => {
		const input = document.activeElement as HTMLInputElement;
		const data = new DataTransfer();
		data.setData('text/plain', '-----BEGIN KEY-----\nabc\n-----END KEY-----');
		input.dispatchEvent(new ClipboardEvent('paste', { clipboardData: data, cancelable: true }));
	});
	await expect(dialog).toContainText('The pasted text has 3 lines, and a variable holds one');
	await expect(value).toHaveValue('');
	await value.fill('x'.repeat(128 * 1024));
	await expect(dialog).toContainText('A variable takes at most 128 KiB with its name');
	await expect(dialog.getByRole('button', { name: 'Save variable' })).toBeDisabled();
	await value.fill('http://proxy.lan:3128');
	await expect(dialog.getByRole('button', { name: 'Save variable' })).toBeEnabled();
});

test('a file is sent as typed or as the bytes uploaded, up to the limit', async ({ page }) => {
	const sent: unknown[] = [];
	await page.route('**/api/setup', async (route) => {
		if (route.request().method() === 'GET') {
			return route.fulfill({ json: [] });
		}
		sent.push(route.request().postDataJSON());
		return route.fulfill({ status: 400, json: { error: 'refused to stay on the page' } });
	});
	await page.goto('/setup');
	await page.getByRole('button', { name: 'Add file' }).first().click();
	const dialog = page.getByRole('dialog', { name: 'Add a file' });
	await dialog.getByLabel('Path in the home').fill('~/.config/corral/x');
	await expect(dialog).toContainText('Corral keeps the sandbox’s own files');
	await dialog.getByLabel('Path in the home').fill('$HOME/bin/tool.sh');
	await dialog.getByLabel('Value', { exact: true }).fill('#!/bin/sh\necho "{{ GH_TOKEN }}"\n');
	await dialog.getByRole('switch', { name: 'Template' }).click();
	await dialog.getByRole('switch', { name: 'Executable' }).click();
	await dialog.getByRole('button', { name: 'Save file' }).click();
	await expect(dialog.getByRole('alert')).toHaveText('refused to stay on the page');

	await dialog.getByLabel('Upload a file').setInputFiles({
		name: 'too-big.bin',
		mimeType: 'application/octet-stream',
		buffer: Buffer.alloc(256 * 1024 + 1)
	});
	await expect(dialog).toContainText('more than the 256 KiB a file may hold');
	await dialog.getByLabel('Upload a file').setInputFiles({
		name: 'key.bin',
		mimeType: 'application/octet-stream',
		buffer: Buffer.from([0, 159, 146, 150, 255])
	});
	await expect(dialog).toContainText('key.bin');
	await dialog.getByRole('switch', { name: 'Template' }).click();
	await dialog.getByRole('switch', { name: 'Executable' }).click();
	await dialog.getByRole('switch', { name: 'Secret' }).click();
	await dialog.getByRole('button', { name: 'Save file' }).click();
	await expect(dialog.getByRole('alert')).toBeVisible();

	expect(sent).toEqual([
		{
			kind: 'file',
			target: 'bin/tool.sh',
			secret: false,
			template: true,
			executable: true,
			applyToNew: true,
			value: { text: '#!/bin/sh\necho "{{ GH_TOKEN }}"\n' }
		},
		{
			kind: 'file',
			target: 'bin/tool.sh',
			secret: true,
			template: false,
			executable: false,
			applyToNew: true,
			value: { base64: 'AJ+Slv8=' }
		}
	]);
});

test('an entry changes name, kind and secrecy, and an untouched value saves nothing', async ({
	page
}) => {
	const file = entry({
		id: '6c4d5e6f-7a8b-4c9d-0e1f-2a3b4c5d6e7f',
		kind: 'file',
		target: 'notes',
		secret: true,
		value: { setAt, size: 411, shown: 'hidden' },
		sandboxes: [{ sandbox: 'main', overridden: true, unresolved: [] }]
	});
	const calls: Call[] = [];
	await record(
		page,
		`/api/setup/${file.id}`,
		(route) => {
			if (route.request().method() === 'GET') {
				return { json: file };
			}
			const { value, ...change } = route.request().postDataJSON();
			return { json: { ...file, ...change, value: value ? file.value : file.value } };
		},
		calls
	);
	await page.route('**/api/sandboxes', (route) => route.fulfill({ json: [] }));
	await page.goto(`/setup/${file.id}`);
	await expect(page.getByRole('img', { name: 'Secret value, 411 bytes' })).toBeVisible();

	await page.getByRole('button', { name: 'Replace value' }).click();
	const replace = page.getByRole('dialog', { name: 'Replace ~/notes' });
	await expect(replace).toContainText('Now secret, 411 bytes.');
	await expect(replace.getByRole('button', { name: 'Save value' })).toBeDisabled();
	const text = replace.getByLabel('Value', { exact: true });
	await text.fill('new key');
	await expect(text).toHaveCSS('-webkit-text-security', 'disc');
	await replace.getByRole('button', { name: 'Show the value' }).click();
	await expect(text).toHaveCSS('-webkit-text-security', 'none');
	await replace.getByRole('button', { name: 'Save value' }).click();
	await expect(replace).toBeHidden();

	await page.getByRole('button', { name: 'Edit', exact: true }).click();
	const edit = page.getByRole('dialog', { name: 'Edit ~/notes' });
	await edit.getByRole('radio', { name: 'Variable' }).click();
	await edit.getByLabel('Name').fill('NOTES');
	await edit.getByRole('switch', { name: 'Secret' }).click();
	await expect(edit).toContainText('suits a plain variable');
	await edit.getByRole('button', { name: 'Save changes' }).click();
	await expect(edit).toBeHidden();

	expect(calls.filter((call) => call.method === 'PUT').map((call) => call.body)).toEqual([
		{
			kind: 'file',
			target: 'notes',
			secret: true,
			template: false,
			executable: false,
			applyToNew: true,
			value: { text: 'new key' }
		},
		{
			kind: 'variable',
			target: 'NOTES',
			secret: false,
			template: false,
			executable: false,
			applyToNew: true
		}
	]);
});

test('a sandbox attaches several entries, overrides and detaches them', async ({ page }) => {
	const token = entry({ sandboxes: [{ sandbox: 'main', overridden: false, unresolved: [] }] });
	const notes = entry({
		id: '6c4d5e6f-7a8b-4c9d-0e1f-2a3b4c5d6e7f',
		kind: 'file',
		target: 'notes',
		secret: false,
		value: { setAt, size: 2, shown: 'text', text: 'hi' }
	});
	const name = entry({
		id: '7d5e6f7a-8b9c-4d0e-1f2a-3b4c5d6e7f8a',
		target: 'GIT_NAME',
		secret: false,
		value: { setAt, size: 4, shown: 'text', text: 'Adam' }
	});
	let attached: AttachedEntry[] = [{ entry: token, override: null }];
	await page.route('**/api/sandboxes/main', (route) => route.fulfill({ json: main }));
	await page.route('**/api/setup', (route) => route.fulfill({ json: [token, notes, name] }));
	await page.route('**/api/sandboxes/main/setup', (route) => route.fulfill({ json: attached }));
	const calls: Call[] = [];
	await record(
		page,
		'/api/sandboxes/main/setup/*',
		(route) => {
			const request = route.request();
			const id = new URL(request.url()).pathname.split('/').at(-1);
			if (request.method() === 'PUT') {
				const added = [notes, name].find((held) => held.id === id)!;
				attached = [...attached, { entry: added, override: null }];
			} else {
				attached = attached.filter((held) => held.entry.id !== id);
			}
			return { status: 204 };
		},
		calls
	);
	await record(
		page,
		'/api/sandboxes/main/setup/*/override',
		(route) => {
			const overriding = route.request().method() === 'PUT';
			attached = attached.map((held) =>
				held.entry.id === token.id
					? {
							...held,
							override: overriding
								? { setAt, size: 8, shown: 'masked', length: 8, head: null, tail: null }
								: null
						}
					: held
			);
			return { status: 204 };
		},
		calls
	);
	await page.goto('/sandboxes/main');
	const section = page.getByRole('list', { name: 'Attached to main' });
	await expect(section.getByRole('listitem')).toHaveCount(1);

	await page.getByRole('button', { name: 'Attach', exact: true }).click();
	const dialog = page.getByRole('dialog', { name: 'Attach to main' });
	await expect(dialog.getByRole('listitem')).toHaveCount(2);
	await dialog.getByRole('checkbox').first().click();
	await dialog.getByRole('checkbox').last().click();
	await dialog.getByRole('button', { name: 'Attach 2' }).click();
	await expect(dialog).toBeHidden();
	await expect(section.getByRole('listitem')).toHaveCount(3);

	await page.getByRole('button', { name: 'Override for this sandbox: GH_TOKEN' }).click();
	const own = page.getByRole('dialog', { name: 'Override GH_TOKEN for main' });
	await own.getByLabel('Value', { exact: true }).fill('ghp_main');
	await own.getByRole('button', { name: 'Save override' }).click();
	await expect(section.getByText('Override', { exact: true })).toBeVisible();
	await expect(section).toContainText('8 characters');
	await page.getByRole('button', { name: 'Remove override of GH_TOKEN' }).click();
	await page.getByRole('alertdialog').getByRole('button', { name: 'Remove override' }).click();
	await expect(section.getByText('Override', { exact: true })).toBeHidden();
	await page.getByRole('button', { name: 'Detach ~/notes' }).click();
	await page.getByRole('alertdialog').getByRole('button', { name: 'Detach' }).click();
	await expect(section.getByRole('listitem')).toHaveCount(2);

	expect(calls.map(({ method, path, body }) => [method, path.split('/').slice(5), body])).toEqual([
		['PUT', [name.id], null],
		['PUT', [notes.id], null],
		['PUT', [token.id, 'override'], { text: 'ghp_main' }],
		['DELETE', [token.id, 'override'], null],
		['DELETE', [notes.id], null]
	]);
});

test('shared entries come first, a required one locked, and yours says what it replaces', async ({
	page
}) => {
	const shadowed = { ...openai, shadowed: true };
	const mine = entry({
		id: '5d3e4f5a-6b7c-4d8e-9f0a-1b2c3d4e5f6a',
		target: 'OPENAI_BASE_URL',
		secret: false,
		shadows: true,
		value: { setAt, size: 24, shown: 'text', text: 'https://openai.local.lan' }
	});
	await page.route('**/api/setup', (route) =>
		route.fulfill({ json: [gateway, gatewayToken, shadowed, mine, entry({})] })
	);
	await page.route(`**/api/setup/${gateway.id}`, (route) => route.fulfill({ json: gateway }));
	await page.route(`**/api/setup/${openai.id}`, (route) => route.fulfill({ json: openai }));
	await page.route('**/api/sandboxes', (route) => route.fulfill({ json: [main] }));
	await page.goto('/setup');

	await expect(page.getByRole('heading', { level: 2 })).toHaveText(['Presets', 'Shared', 'Yours']);
	const shared = page.getByRole('list', { name: 'Shared' });
	const yours = page.getByRole('list', { name: 'Yours' });
	const required = shared.getByRole('link', { name: /ANTHROPIC_BASE_URL/ });
	await expect(required).toContainText('Required');
	await expect(required).toContainText(
		'https://llm.example.lan · set by the admin for every sandbox'
	);
	await expect(shared.getByRole('link', { name: /ANTHROPIC_AUTH_TOKEN/ })).toContainText(
		'Secret, 23 bytes · set by the admin for every sandbox'
	);
	const offered = shared.getByRole('link', { name: /OPENAI_BASE_URL/ });
	await expect(offered).toContainText('Offered');
	await expect(offered).toContainText(
		'https://openai.example.lan · offered to every sandbox · attached to no sandbox · new sandboxes get it · yours replaces it'
	);
	await expect(yours.getByRole('listitem')).toHaveCount(2);
	await expect(yours.getByRole('link', { name: /OPENAI_BASE_URL/ })).toContainText(
		'Yours, in place of the shared one'
	);
	await expect(page.getByText('Yours, in place of the shared one')).toHaveCount(1);

	await required.click();
	await expect(page.getByRole('heading', { name: 'ANTHROPIC_BASE_URL' })).toBeVisible();
	await expect(page.getByText('Set by the admin in Settings > Shared setup')).toBeVisible();
	await expect(page.getByText('Every sandbox gets it.')).toBeVisible();
	for (const name of ['Edit', 'Duplicate', 'Edit value', 'Replace value', 'Delete variable']) {
		await expect(page.getByRole('button', { name, exact: true })).toHaveCount(0);
	}
	await expect(page.getByRole('button', { name: /^(Attach|Detach)/ })).toHaveCount(0);

	await page.goto(`/setup/${openai.id}`);
	await expect(page.getByText('Set by the admin in Settings > Shared setup')).toBeVisible();
	await expect(page.getByRole('button', { name: 'Attach to main' })).toBeVisible();
	await expect(page.getByRole('button', { name: 'Edit', exact: true })).toHaveCount(0);
});

test('a sandbox gets each required entry, locked, and attaches an offered one', async ({
	page
}) => {
	const token = entry({});
	let attached: AttachedEntry[] = [
		{ entry: gateway, override: null },
		{ entry: token, override: null }
	];
	await page.route('**/api/sandboxes/main', (route) => route.fulfill({ json: main }));
	await page.route('**/api/setup', (route) => route.fulfill({ json: [gateway, openai, token] }));
	await page.route('**/api/sandboxes/main/setup', (route) => route.fulfill({ json: attached }));
	const calls: Call[] = [];
	await record(
		page,
		'/api/sandboxes/main/setup/*',
		() => {
			attached = [gateway, openai, token].map((entry) => ({ entry, override: null }));
			return { status: 204 };
		},
		calls
	);
	await page.goto('/sandboxes/main');
	const section = page.getByRole('list', { name: 'Attached to main' });
	await expect(section.getByRole('listitem')).toHaveCount(2);
	await expect(section.getByRole('listitem').first()).toContainText('ANTHROPIC_BASE_URL');
	await expect(section.getByRole('listitem').first()).toContainText('Required');
	await expect(page.getByRole('button', { name: 'Detach ANTHROPIC_BASE_URL' })).toHaveCount(0);
	await expect(
		page.getByRole('button', { name: 'Override for this sandbox: ANTHROPIC_BASE_URL' })
	).toHaveCount(0);

	await page.getByRole('button', { name: 'Attach', exact: true }).click();
	const dialog = page.getByRole('dialog', { name: 'Attach to main' });
	await expect(dialog.getByRole('listitem')).toHaveCount(1);
	await expect(dialog.getByRole('listitem')).toContainText('OPENAI_BASE_URL');
	await expect(dialog.getByRole('listitem')).toContainText('Offered');
	await dialog.getByRole('checkbox').click();
	await dialog.getByRole('button', { name: 'Attach', exact: true }).click();
	await expect(dialog).toBeHidden();

	await expect(section.getByRole('listitem')).toHaveCount(3);
	await expect(section.getByRole('listitem').nth(1)).toContainText('Offered');
	await expect(page.getByRole('button', { name: 'Detach OPENAI_BASE_URL' })).toBeVisible();
	expect(calls.map(({ method, path }) => [method, path.split('/').at(-1)])).toEqual([
		['PUT', openai.id]
	]);
});

test('files count their lines, keep their case, and are named by their path', async ({ page }) => {
	const short = entry({
		id: '8e6f7a8b-9c0d-4e1f-2a3b-4c5d6e7f8a9b',
		kind: 'file',
		target: '.hushlogin',
		secret: false,
		value: { setAt, size: 4, shown: 'text', text: 'one\n' }
	});
	const text = Array.from({ length: 20 }, (_, index) => `line ${index + 1}\n`).join('');
	const long = entry({
		id: '9f7a8b9c-0d1e-4f2a-3b4c-5d6e7f8a9b0c',
		kind: 'file',
		target: 'notes/t.txt',
		secret: false,
		value: { setAt, size: text.length, shown: 'text', text }
	});
	const lower = entry({
		id: '0a8b9c0d-1e2f-4a3b-4c5d-6e7f8a9b0c1d',
		target: 'GIT_EDITOR',
		secret: false,
		value: { setAt, size: 3, shown: 'text', text: 'vim' }
	});
	const empty = entry({
		id: '1b9c0d1e-2f3a-4b4c-5d6e-7f8a9b0c1d2e',
		target: 'EMPTY_VALUE',
		secret: false,
		value: { setAt, size: 0, shown: 'text', text: '' }
	});
	await page.route('**/api/setup', (route) => route.fulfill({ json: [short, long, lower, empty] }));
	await page.route(`**/api/setup/${long.id}`, (route) => route.fulfill({ json: long }));
	await page.route('**/api/sandboxes', (route) => route.fulfill({ json: [] }));
	await page.goto('/setup');
	const listed = (target: string) => page.getByRole('link', { name: new RegExp(target) });
	await expect(listed('hushlogin')).toContainText('1 line · attached to no sandbox');
	await expect(listed('notes')).toContainText('20 lines · attached to no sandbox');
	const description = listed('GIT_EDITOR').getByText('vim · attached to no sandbox');
	expect(
		await description.evaluate(
			(element) => getComputedStyle(element, '::first-letter').textTransform
		)
	).toBe('none');
	await expect(listed('EMPTY_VALUE')).toContainText(
		'Attached to no sandbox · new sandboxes get it'
	);

	await page.goto(`/setup/${long.id}`);
	await expect(page.getByRole('button', { name: 'Show all 20 lines' })).toBeVisible();
	await page.getByRole('button', { name: 'Duplicate' }).click();
	const duplicate = page.getByRole('dialog', { name: 'Duplicate ~/notes/t.txt' });
	await expect(duplicate.getByLabel('Path in the home')).toHaveValue('~/notes/t.txt.copy');
	await duplicate.getByRole('button', { name: 'Cancel' }).click();
	await page.getByRole('button', { name: 'Delete file' }).click();
	const confirm = page.getByRole('alertdialog', { name: 'Delete ~/notes/t.txt?' });
	const remove = confirm.getByRole('button', { name: 'Delete file' });
	await confirm.getByLabel('Type ~/notes/t.txt to confirm').fill('notes/t.txt');
	await expect(remove).toBeDisabled();
	await confirm.getByLabel('Type ~/notes/t.txt to confirm').fill('~/notes/t.txt');
	await expect(remove).toBeEnabled();
});

test('an import names how many variables it would import', async ({ page }) => {
	await page.route('**/api/setup', (route) => route.fulfill({ json: [] }));
	await page.goto('/setup');
	await page.getByRole('button', { name: 'Import .env' }).first().click();
	const dialog = page.getByRole('dialog', { name: 'Import variables' });
	const button = (name: string) => dialog.getByRole('button', { name, exact: true });
	await expect(button('Import')).toBeDisabled();
	await dialog.getByLabel('Variables').fill('EDITOR=vim');
	await expect(button('Import 1 variable')).toBeEnabled();
	await dialog.getByLabel('Variables').fill('EDITOR=vim\nPAGER=less');
	await expect(button('Import 2 variables')).toBeEnabled();
});

const presets = ['LLM providers', 'Git identity', 'MCP servers', 'Dotfiles'];

test('each preset says it is not set up on an empty setup, and opens its form', async ({
	page
}) => {
	await page.route('**/api/setup', (route) => route.fulfill({ json: [] }));
	await page.goto('/setup');
	const cards = page.getByRole('list', { name: 'Presets' }).getByRole('listitem');
	await expect(cards).toHaveCount(4);
	for (const [index, preset] of presets.entries()) {
		await expect(cards.nth(index).getByRole('heading', { level: 3 })).toHaveText(preset);
		await expect(cards.nth(index)).toContainText('Not set up');
		await expect(cards.nth(index).getByRole('button')).toHaveAccessibleName(`Set up ${preset}`);
	}

	await page.getByRole('button', { name: 'Set up Git identity' }).click();
	await expect(page.getByRole('dialog', { name: 'Git identity' })).toBeVisible();
});

test('each preset sums up the entries it reads back', async ({ page }) => {
	const text = (value: string): SetupEntry['value'] => ({
		setAt,
		size: value.length,
		shown: 'text',
		text: value
	});
	const hidden: SetupEntry['value'] = { setAt, size: 40, shown: 'hidden' };
	const own: Partial<SetupEntry>[] = [
		{ target: 'ANTHROPIC_BASE_URL', secret: false, value: text('https://llm.home.example') },
		{ target: 'ANTHROPIC_API_KEY' },
		{ target: 'ANTHROPIC_AUTH_TOKEN' },
		{ target: 'OPENAI_API_KEY' },
		{
			kind: 'file',
			target: '.gitconfig',
			secret: false,
			value: text(
				'[user]\n\tname = Ada Lovelace\n\temail = ada@example.com\n[credential]\n\thelper = store\n[credential "https://github.com"]\n\tusername = ada\n[init]\n\tdefaultBranch = main\n'
			)
		},
		{ kind: 'file', target: '.git-credentials', value: hidden },
		{
			kind: 'file',
			target: '.config/corral-agents/mcp.json',
			secret: false,
			value: text(
				JSON.stringify({
					fetch: { command: 'uvx', args: ['mcp-server-fetch'], env: {} },
					github: { type: 'http', url: 'https://api.githubcopilot.com/mcp/', headers: {} },
					docs: { type: 'http', url: 'https://docs.example/mcp', headers: {} }
				})
			)
		},
		{
			kind: 'file',
			target: '.config/corral-agents/dotfiles.json',
			secret: false,
			value: text('{"url": "https://github.com/ada/dotfiles.git", "branch": "main"}')
		}
	];
	const entries = own.map((extra, index) =>
		entry({ id: `0b8c1d2e-3f4a-4b5c-8d6e-7f8a9b0c1d${String(index).padStart(2, '0')}`, ...extra })
	);
	await page.route('**/api/setup', (route) => route.fulfill({ json: entries }));
	await page.goto('/setup');
	const cards = page.getByRole('list', { name: 'Presets' }).getByRole('listitem');
	for (const [index, summary] of [
		'Anthropic through a gateway, OpenAI key set',
		'Ada Lovelace <ada@example.com>, token for github.com',
		'3 servers: fetch, github, docs',
		'github.com/ada/dotfiles, main'
	].entries()) {
		await expect(cards.nth(index)).toContainText(summary);
		await expect(cards.nth(index).getByRole('button')).toHaveAccessibleName(
			`Change ${presets[index]}`
		);
	}
});

const masked: SetupEntry['value'] = {
	setAt,
	size: 12,
	shown: 'masked',
	length: 12,
	head: null,
	tail: null
};

/** Answers the Setup API with `entries`, adding what each create sends, and records the calls. */
async function setupApi(page: Page, entries: SetupEntry[], calls: Call[]) {
	let listed = entries;
	await record(
		page,
		'/api/setup',
		(route) => {
			if (route.request().method() === 'GET') {
				return { json: listed };
			}
			const sent: NewEntry = route.request().postDataJSON();
			const made = entry({
				id: crypto.randomUUID(),
				kind: sent.kind,
				target: sent.target,
				secret: sent.secret,
				value:
					sent.secret || !('text' in sent.value)
						? masked
						: { setAt, size: sent.value.text.length, shown: 'text', text: sent.value.text }
			});
			listed = [...listed, made];
			return { status: 201, json: made };
		},
		calls
	);
}

const sent = (calls: Call[]) =>
	calls
		.filter((call) => call.method !== 'GET')
		.map(({ method, path, body }) => [method, path, body]);

const created = (target: string, secret: boolean, text: string, kind = 'variable') => [
	'POST',
	'/api/setup',
	{ kind, target, secret, applyToNew: true, value: { text } }
];

test('the LLM providers preset saves a key with an address as three variables, then hides it', async ({
	page
}) => {
	const calls: Call[] = [];
	await setupApi(page, [], calls);
	await page.goto('/setup');
	await page.getByRole('button', { name: 'Set up LLM providers' }).click();
	const dialog = page.getByRole('dialog', { name: 'LLM providers' });
	const anthropic = dialog.getByRole('group', { name: 'Anthropic' });
	await expect(anthropic.getByLabel('Base URL')).toHaveAttribute(
		'placeholder',
		'https://llm.home.example'
	);
	await expect(anthropic).toContainText('Optional. Used exactly as typed, for a gateway or proxy.');
	await anthropic.getByLabel('Base URL').fill('https://llm.home.example');
	await anthropic.getByLabel('Key', { exact: true }).fill('sk-ant-typed');
	await dialog.getByRole('button', { name: 'Save providers' }).click();
	await expect(dialog).toBeHidden();
	await expect(page.getByText('Saved the LLM providers')).toBeVisible();
	expect(sent(calls)).toEqual([
		created('ANTHROPIC_BASE_URL', false, 'https://llm.home.example'),
		created('ANTHROPIC_API_KEY', true, 'sk-ant-typed'),
		created('ANTHROPIC_AUTH_TOKEN', true, 'sk-ant-typed')
	]);

	await expect(page.getByRole('list', { name: 'Presets' })).toContainText(
		'Anthropic through a gateway'
	);
	await page.getByRole('button', { name: 'Change LLM providers' }).click();
	await expect(anthropic.getByLabel('Base URL')).toHaveValue('https://llm.home.example');
	await expect(anthropic).toContainText('Set, hidden');
	for (const input of await dialog.locator('input').all()) {
		await expect(input).not.toHaveValue(/sk-ant/);
	}
	await anthropic.getByRole('button', { name: 'Replace the Anthropic key' }).click();
	await expect(anthropic.getByLabel('Key', { exact: true })).toBeFocused();
	await expect(anthropic.getByLabel('Key', { exact: true })).toHaveValue('');
	await expect(dialog.getByRole('button', { name: 'Save providers' })).toBeDisabled();
});

test("one provider's address fills the others, which stay editable", async ({ page }) => {
	await page.route('**/api/setup', (route) => route.fulfill({ json: [] }));
	await page.goto('/setup');
	await page.getByRole('button', { name: 'Set up LLM providers' }).click();
	const dialog = page.getByRole('dialog', { name: 'LLM providers' });
	const anthropic = dialog.getByRole('group', { name: 'Anthropic' });
	const openai = dialog.getByRole('group', { name: 'OpenAI' });
	const copy = (group: Locator) =>
		group.getByRole('button', { name: 'Use this address for every provider' });
	await expect(copy(dialog)).toHaveCount(0);

	await openai.getByLabel('Base URL').fill('https://openai.home.example');
	await expect(copy(openai)).toBeVisible();
	await anthropic.getByLabel('Base URL').fill('https://llm.home.example');
	await expect(copy(dialog)).toHaveCount(1);
	await copy(anthropic).click();
	await expect(openai.getByLabel('Base URL')).toHaveValue('https://llm.home.example');
	await openai.getByLabel('Base URL').fill('https://llm.home.example/openai');
	await expect(anthropic.getByLabel('Base URL')).toHaveValue('https://llm.home.example');
});

test('with the addresses set by the admin, the LLM providers preset saves the keys only', async ({
	page
}) => {
	const openaiGateway = entry({
		id: '5e4f5a6b-7c8d-4e9f-8a0b-1c2d3e4f5a6b',
		target: 'OPENAI_BASE_URL',
		secret: false,
		shared: 'required',
		value: { setAt, size: 30, shown: 'text', text: 'https://llm.example.lan/openai' }
	});
	const key = entry({ target: 'ANTHROPIC_API_KEY', value: masked });
	const calls: Call[] = [];
	await setupApi(page, [gateway, openaiGateway, key], calls);
	await record(page, `/api/setup/${key.id}`, () => ({ json: key }), calls);
	await page.goto('/setup');
	await page.getByRole('button', { name: 'Change LLM providers' }).click();
	const dialog = page.getByRole('dialog', { name: 'LLM providers' });
	const anthropic = dialog.getByRole('group', { name: 'Anthropic' });
	const openai = dialog.getByRole('group', { name: 'OpenAI' });
	for (const [group, url] of [
		[anthropic, 'https://llm.example.lan'],
		[openai, 'https://llm.example.lan/openai']
	] as const) {
		await expect(group.getByLabel('Base URL')).toHaveValue(url);
		await expect(group.getByLabel('Base URL')).toBeDisabled();
		await expect(group).toContainText('Set by the admin');
	}
	await expect(
		dialog.getByRole('button', { name: 'Use this address for every provider' })
	).toHaveCount(0);

	await expect(anthropic).toContainText('Type the key again so it is also sent as a bearer token.');
	const save = dialog.getByRole('button', { name: 'Save providers' });
	await expect(save).toBeDisabled();
	await anthropic.getByLabel('Key', { exact: true }).fill('sk-ant-again');
	await openai.getByLabel('Key', { exact: true }).fill('sk-openai');
	await save.click();
	await expect(dialog).toBeHidden();
	expect(sent(calls)).toEqual([
		[
			'PUT',
			`/api/setup/${key.id}`,
			{
				kind: 'variable',
				target: 'ANTHROPIC_API_KEY',
				secret: true,
				template: false,
				executable: false,
				applyToNew: true,
				value: { text: 'sk-ant-again' }
			}
		],
		created('ANTHROPIC_AUTH_TOKEN', true, 'sk-ant-again'),
		created('OPENAI_API_KEY', true, 'sk-openai')
	]);
});

test("the LLM providers preset shows Corral's refusal next to the field it names", async ({
	page
}) => {
	const refusal = 'conflict: variable ANTHROPIC_AUTH_TOKEN is set for every sandbox by the admin';
	await page.route('**/api/setup', (route) => {
		const request = route.request();
		if (request.method() === 'GET') {
			return route.fulfill({ json: [gateway, gatewayToken] });
		}
		return request.postDataJSON().target === 'ANTHROPIC_AUTH_TOKEN'
			? route.fulfill({ status: 409, json: { error: refusal } })
			: route.fulfill({ status: 201, json: entry({ target: 'ANTHROPIC_API_KEY' }) });
	});
	await page.goto('/setup');
	await page.getByRole('button', { name: 'Set up LLM providers' }).click();
	const dialog = page.getByRole('dialog', { name: 'LLM providers' });
	const anthropic = dialog.getByRole('group', { name: 'Anthropic' });
	await anthropic.getByLabel('Key', { exact: true }).fill('sk-ant-typed');
	await dialog.getByRole('button', { name: 'Save providers' }).click();
	await expect(anthropic.getByRole('alert')).toHaveText(refusal);
	await expect(anthropic.getByLabel('Key', { exact: true })).toHaveAttribute(
		'aria-invalid',
		'true'
	);
	await expect(dialog.getByRole('alert')).toHaveCount(1);
});

const identity = '[user]\n\tname = Ada Lovelace\n\temail = ada@example.com\n';

test('the git identity preset writes ~/.gitconfig and the token, then reads them back', async ({
	page
}) => {
	const calls: Call[] = [];
	await setupApi(page, [], calls);
	await page.goto('/setup');
	await page.getByRole('button', { name: 'Set up Git identity' }).click();
	const dialog = page.getByRole('dialog', { name: 'Git identity' });
	await expect(dialog).toContainText(
		'Written as ~/.gitconfig, with the token or SSH key git uses to reach your repositories.'
	);
	await expect(dialog.getByRole('radio', { name: 'Nothing' })).toBeChecked();
	await dialog.getByLabel('Name', { exact: true }).fill('Ada Lovelace');
	await dialog.getByLabel('Email', { exact: true }).fill('ada@example.com');
	await dialog.getByRole('radio', { name: 'A token' }).click();
	await expect(dialog.getByLabel('Host', { exact: true })).toHaveAttribute(
		'placeholder',
		'github.com'
	);
	await expect(dialog.getByLabel('User', { exact: true })).toHaveAttribute('placeholder', 'git');
	await dialog.getByLabel('Host', { exact: true }).fill('github.com');
	await dialog.getByLabel('User', { exact: true }).fill('ada');
	await dialog.getByLabel('Token', { exact: true }).fill('ghp_s3cret');
	await dialog.getByRole('button', { name: 'Save identity' }).click();
	await expect(dialog).toBeHidden();
	await expect(page.getByText('Saved your git identity')).toBeVisible();
	expect(sent(calls)).toEqual([
		created(
			'.gitconfig',
			false,
			`${identity}[credential]\n\thelper = store\n[credential "https://github.com"]\n\tusername = ada\n[init]\n\tdefaultBranch = main\n`,
			'file'
		),
		created('.git-credentials', true, 'https://ada:ghp_s3cret@github.com\n', 'file')
	]);

	await expect(page.getByRole('list', { name: 'Presets' })).toContainText(
		'Ada Lovelace <ada@example.com>, token for github.com'
	);
	await page.getByRole('button', { name: 'Change Git identity' }).click();
	await expect(dialog.getByLabel('Name', { exact: true })).toHaveValue('Ada Lovelace');
	await expect(dialog.getByLabel('Email', { exact: true })).toHaveValue('ada@example.com');
	await expect(dialog.getByRole('radio', { name: 'A token' })).toBeChecked();
	await expect(dialog.getByLabel('Host', { exact: true })).toHaveValue('github.com');
	await expect(dialog.getByLabel('User', { exact: true })).toHaveValue('ada');
	await expect(dialog).toContainText('Set, hidden');
	for (const input of await dialog.locator('input, textarea').all()) {
		await expect(input).not.toHaveValue(/s3cret/);
	}
	const save = dialog.getByRole('button', { name: 'Save identity' });
	await expect(save).toBeDisabled();
	await dialog.getByLabel('Host', { exact: true }).fill('gitlab.com');
	await expect(dialog.getByLabel('Token', { exact: true })).toHaveAccessibleDescription(
		'Type the token again to save it for this host and user.'
	);
	await expect(save).toBeDisabled();
	await dialog.getByLabel('Token', { exact: true }).fill('glpat-s3cret');
	await expect(save).toBeEnabled();
});

test('the git identity preset sends an SSH key as a secret file, and never shows it again', async ({
	page
}) => {
	const privateKey =
		'-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjE=\n-----END OPENSSH PRIVATE KEY-----\n';
	const publicKey = 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5 ada@laptop';
	const calls: Call[] = [];
	await setupApi(page, [], calls);
	await page.goto('/setup');
	await page.getByRole('button', { name: 'Set up Git identity' }).click();
	const dialog = page.getByRole('dialog', { name: 'Git identity' });
	await dialog.getByLabel('Name', { exact: true }).fill('Ada Lovelace');
	await dialog.getByLabel('Email', { exact: true }).fill('ada@example.com');
	const ssh = dialog.getByRole('radio', { name: 'An SSH key' });
	await expect(ssh).toHaveAccessibleDescription(
		'Written to ~/.ssh/id_ed25519, replacing a key kept there.'
	);
	await ssh.click();
	const save = dialog.getByRole('button', { name: 'Save identity' });
	await expect(save).toBeDisabled();
	await dialog.locator('#git-private-key-upload').setInputFiles({
		name: 'id_ed25519',
		mimeType: 'application/octet-stream',
		buffer: Buffer.from(privateKey)
	});
	await dialog.getByLabel('Public key', { exact: true }).fill(publicKey);
	await save.click();
	await expect(dialog).toBeHidden();
	expect(sent(calls)).toEqual([
		created('.gitconfig', false, `${identity}[init]\n\tdefaultBranch = main\n`, 'file'),
		created('.ssh/id_ed25519', true, privateKey, 'file'),
		created('.ssh/id_ed25519.pub', false, `${publicKey}\n`, 'file')
	]);

	await expect(page.getByRole('list', { name: 'Presets' })).toContainText(
		'Ada Lovelace <ada@example.com>, SSH key'
	);
	await page.getByRole('button', { name: 'Change Git identity' }).click();
	await expect(dialog.getByLabel('Name', { exact: true })).toHaveValue('Ada Lovelace');
	await expect(ssh).toBeChecked();
	await expect(dialog).toContainText('Set, hidden');
	await expect(dialog.locator('textarea')).toHaveCount(0);
	await dialog.getByRole('button', { name: 'Replace the SSH key' }).click();
	await expect(dialog.getByLabel('Private key', { exact: true })).toBeFocused();
	await expect(dialog.getByLabel('Private key', { exact: true })).toHaveValue('');
});

test("the git identity preset shows Corral's refusal next to the field it names", async ({
	page
}) => {
	const refusal = 'conflict: file .git-credentials is set for every sandbox by the admin';
	await page.route('**/api/setup', (route) => {
		const request = route.request();
		if (request.method() === 'GET') {
			return route.fulfill({ json: [] });
		}
		return request.postDataJSON().target === '.git-credentials'
			? route.fulfill({ status: 409, json: { error: refusal } })
			: route.fulfill({ status: 201, json: entry({ kind: 'file', target: '.gitconfig' }) });
	});
	await page.goto('/setup');
	await page.getByRole('button', { name: 'Set up Git identity' }).click();
	const dialog = page.getByRole('dialog', { name: 'Git identity' });
	await dialog.getByRole('radio', { name: 'A token' }).click();
	await dialog.getByLabel('Host', { exact: true }).fill('github.com');
	await dialog.getByLabel('User', { exact: true }).fill('ada');
	await dialog.getByLabel('Token', { exact: true }).fill('ghp_s3cret');
	await dialog.getByRole('button', { name: 'Save identity' }).click();
	const token = dialog.getByLabel('Token', { exact: true });
	await expect(token).toHaveAttribute('aria-invalid', 'true');
	await expect(token).toHaveAccessibleDescription(refusal);
	await expect(dialog.getByRole('alert')).toHaveCount(1);
});

const servers = '.config/corral-agents/mcp.json';

test('the MCP servers preset writes a command and a URL server as mcp.json, then reads them back', async ({
	page
}) => {
	const calls: Call[] = [];
	await setupApi(page, [], calls);
	await page.goto('/setup');
	await page.getByRole('button', { name: 'Set up MCP servers' }).click();
	const dialog = page.getByRole('dialog', { name: 'MCP servers' });
	await expect(dialog).toContainText(
		'Applied to Claude Code, Codex, Pi and oh-my-pi at each start, as their user-scope servers.'
	);
	const first = dialog.getByRole('group', { name: 'Server 1' });
	await expect(first.getByRole('radio', { name: 'A command' })).toBeChecked();
	const name = first.getByLabel('Name', { exact: true });
	await expect(name).toHaveAccessibleDescription('Letters, digits, - and _.');
	await name.fill('my fetch');
	await expect(name).toHaveAttribute('aria-invalid', 'true');
	await expect(first.getByText('Letters, digits, - and _.')).toBeVisible();
	await name.fill('fetch');
	await expect(name).not.toHaveAttribute('aria-invalid');
	await first.getByLabel('Command', { exact: true }).fill('uvx');
	await first
		.getByLabel('Arguments', { exact: true })
		.fill('mcp-server-fetch\n--ignore-robots-txt');
	await first.getByLabel('Variables', { exact: true }).fill('FETCH_TIMEOUT=30');

	await dialog.getByRole('button', { name: 'Add server' }).click();
	const second = dialog.getByRole('group', { name: 'Server 2' });
	await expect(second.getByLabel('Name', { exact: true })).toBeFocused();
	await second.getByLabel('Name', { exact: true }).fill('fetch');
	await expect(second.getByLabel('Name', { exact: true })).toHaveAccessibleDescription(
		'Another server has this name.'
	);
	const save = dialog.getByRole('button', { name: 'Save servers' });
	await expect(save).toBeDisabled();
	await second.getByLabel('Name', { exact: true }).fill('docs');
	await second.getByRole('radio', { name: 'A URL' }).click();
	await second.getByLabel('URL', { exact: true }).fill('https://docs.example/mcp');
	await second.getByLabel('Headers', { exact: true }).fill('X-Team: platform');
	await save.click();
	await expect(dialog).toBeHidden();
	await expect(page.getByText('Saved the MCP servers')).toBeVisible();
	const file = {
		fetch: {
			command: 'uvx',
			args: ['mcp-server-fetch', '--ignore-robots-txt'],
			env: { FETCH_TIMEOUT: '30' }
		},
		docs: { type: 'http', url: 'https://docs.example/mcp', headers: { 'X-Team': 'platform' } }
	};
	expect(sent(calls)).toEqual([
		created(servers, false, `${JSON.stringify(file, null, 2)}\n`, 'file')
	]);

	await expect(page.getByRole('list', { name: 'Presets' })).toContainText('2 servers: fetch, docs');
	await page.getByRole('button', { name: 'Change MCP servers' }).click();
	await expect(first.getByLabel('Name', { exact: true })).toHaveValue('fetch');
	await expect(first.getByRole('radio', { name: 'A command' })).toBeChecked();
	await expect(first.getByLabel('Command', { exact: true })).toHaveValue('uvx');
	await expect(first.getByLabel('Arguments', { exact: true })).toHaveValue(
		'mcp-server-fetch\n--ignore-robots-txt'
	);
	await expect(first.getByLabel('Variables', { exact: true })).toHaveValue('FETCH_TIMEOUT=30');
	await expect(second.getByLabel('Name', { exact: true })).toHaveValue('docs');
	await expect(second.getByRole('radio', { name: 'A URL' })).toBeChecked();
	await expect(second.getByLabel('URL', { exact: true })).toHaveValue('https://docs.example/mcp');
	await expect(second.getByLabel('Headers', { exact: true })).toHaveValue('X-Team: platform');
	await expect(dialog.getByRole('switch', { name: 'Secret' })).not.toBeChecked();
	await expect(save).toBeDisabled();
});

test('with Secret on, the MCP servers preset sends a secret file, and only offers to replace it', async ({
	page
}) => {
	const calls: Call[] = [];
	await setupApi(page, [], calls);
	await page.goto('/setup');
	await page.getByRole('button', { name: 'Set up MCP servers' }).click();
	const dialog = page.getByRole('dialog', { name: 'MCP servers' });
	const first = dialog.getByRole('group', { name: 'Server 1' });
	await first.getByLabel('Name', { exact: true }).fill('github');
	await first.getByRole('radio', { name: 'A URL' }).click();
	await first.getByLabel('URL', { exact: true }).fill('https://api.githubcopilot.com/mcp/');
	await first.getByLabel('Headers', { exact: true }).fill('Authorization: Bearer ghp_s3cret');
	const secret = dialog.getByRole('switch', { name: 'Secret' });
	await expect(secret).toHaveAccessibleDescription(
		'Keeps tokens in headers or variables hidden; the list cannot be read back here once it is secret, only replaced.'
	);
	await secret.click();
	await dialog.getByRole('button', { name: 'Save servers' }).click();
	await expect(dialog).toBeHidden();
	const file = {
		github: {
			type: 'http',
			url: 'https://api.githubcopilot.com/mcp/',
			headers: { Authorization: 'Bearer ghp_s3cret' }
		}
	};
	expect(sent(calls)).toEqual([
		created(servers, true, `${JSON.stringify(file, null, 2)}\n`, 'file')
	]);

	const card = page.getByRole('list', { name: 'Presets' }).getByRole('listitem').nth(2);
	await expect(card).toContainText('Secret');
	await page.getByRole('button', { name: 'Change MCP servers' }).click();
	await expect(dialog).toContainText('Set, hidden');
	await expect(dialog.getByRole('group', { name: 'Server 1' })).toHaveCount(0);
	for (const input of await dialog.locator('input, textarea').all()) {
		await expect(input).not.toHaveValue(/s3cret/);
	}
	await expect(secret).toBeChecked();
	await expect(secret).toBeDisabled();
	await expect(dialog.getByRole('button', { name: 'Save servers' })).toBeDisabled();
	await dialog.getByRole('button', { name: 'Replace the servers' }).click();
	await expect(first.getByLabel('Name', { exact: true })).toBeFocused();
	await expect(first.getByLabel('Name', { exact: true })).toHaveValue('');
	await expect(secret).toBeChecked();
	await expect(secret).toBeEnabled();
});

test('an entry lives through Corral: made, renamed, made secret and deleted', async ({ page }) => {
	const suffix = Math.random().toString(36).slice(2, 8);
	const first = `e2e_${suffix}`;
	const renamed = `E2E_${suffix.toUpperCase()}`;
	await page.goto('/setup');
	await page.getByRole('button', { name: 'Add variable' }).first().click();
	const add = page.getByRole('dialog', { name: 'Add a variable' });
	await add.getByLabel('Name').fill(first);
	await add.getByLabel('Value', { exact: true }).fill('sk-live-0123456789abcdef');
	await add.getByRole('switch', { name: 'Apply to new sandboxes' }).click();
	await add.getByRole('button', { name: 'Save variable' }).click();
	await expect(page.getByRole('heading', { name: first, exact: true })).toBeVisible();
	await expect(page.getByText('sk-live-0123456789abcdef')).toBeVisible();

	await page.getByRole('button', { name: 'Edit', exact: true }).click();
	const edit = page.getByRole('dialog', { name: `Edit ${first}` });
	await edit.getByLabel('Name').fill(renamed);
	await edit.getByRole('switch', { name: 'Secret' }).click();
	await edit.getByRole('button', { name: 'Save changes' }).click();
	await expect(page.getByRole('heading', { name: renamed, exact: true })).toBeVisible();
	await expect(page.getByRole('img', { name: 'Secret value, 24 characters' })).toContainText(
		'sk-l••••••••••••ef'
	);

	await page.getByRole('link', { name: 'Setup' }).first().click();
	await page.getByRole('button', { name: 'Add variable' }).first().click();
	const again = page.getByRole('dialog', { name: 'Add a variable' });
	await again.getByLabel('Name').fill(renamed);
	await again.getByLabel('Value', { exact: true }).fill('x');
	await again.getByRole('button', { name: 'Save variable' }).click();
	await expect(again.getByRole('alert')).toContainText(`you already have variable ${renamed}`);
	await again.getByRole('button', { name: 'Cancel' }).click();

	await page.getByRole('link', { name: new RegExp(renamed) }).click();
	await page.getByRole('button', { name: 'Delete variable' }).click();
	const confirm = page.getByRole('alertdialog');
	await confirm.getByLabel(`Type ${renamed} to confirm`).fill(renamed);
	await confirm.getByRole('button', { name: 'Delete variable' }).click();
	await expect(page).toHaveURL('/setup');
	await expect(page.getByRole('link', { name: new RegExp(renamed) })).toHaveCount(0);
});
