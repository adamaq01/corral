import type { Page, Route } from '@playwright/test';
import type { AttachedEntry, EnvironmentEntry, Sandbox } from '../src/lib/api/generated/index.js';
import { expect, test, idle } from './fixtures';

test.use({ role: 'member' });

const setAt = '2026-11-20T17:45:00Z';

function entry(extra: Partial<EnvironmentEntry>): EnvironmentEntry {
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
		...extra
	};
}

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
	await page.route('**/api/environment', async (route) => {
		if (route.request().method() === 'GET') {
			return route.fulfill({ json: [] });
		}
		calls.push({ method: 'POST', path: '', body: route.request().postDataJSON() });
		return route.fulfill({ status: 201, json: entry({}) });
	});
	await page.route(`**/api/environment/${entry({}).id}`, (route) =>
		route.fulfill({ json: entry({}) })
	);
	await page.route('**/api/sandboxes', (route) => route.fulfill({ json: [] }));
	await page.goto('/environment');
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

	await expect(page).toHaveURL(`/environment/${entry({}).id}`);
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
	await page.route('**/api/environment', (route) => route.fulfill({ json: [] }));
	await page.goto('/environment');
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
	await page.route('**/api/environment', async (route) => {
		if (route.request().method() === 'GET') {
			return route.fulfill({ json: [] });
		}
		sent.push(route.request().postDataJSON());
		return route.fulfill({ status: 400, json: { error: 'refused to stay on the page' } });
	});
	await page.goto('/environment');
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
		`/api/environment/${file.id}`,
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
	await page.goto(`/environment/${file.id}`);
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
	const sandbox: Sandbox = {
		name: 'main',
		image: 'dev',
		size: 'small',
		isDefault: true,
		ssh: { username: 'main.bob', host: 'corral.example.lan', port: 48291 },
		createdAt: setAt,
		staleEnvironment: false,
		idle,
		deleteAt: null,
		state: 'running'
	};
	await page.route('**/api/sandboxes/main', (route) => route.fulfill({ json: sandbox }));
	await page.route('**/api/environment', (route) => route.fulfill({ json: [token, notes, name] }));
	await page.route('**/api/sandboxes/main/environment', (route) =>
		route.fulfill({ json: attached })
	);
	const calls: Call[] = [];
	await record(
		page,
		'/api/sandboxes/main/environment/*',
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
		'/api/sandboxes/main/environment/*/override',
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
	await page.route('**/api/environment', (route) =>
		route.fulfill({ json: [short, long, lower, empty] })
	);
	await page.route(`**/api/environment/${long.id}`, (route) => route.fulfill({ json: long }));
	await page.route('**/api/sandboxes', (route) => route.fulfill({ json: [] }));
	await page.goto('/environment');
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

	await page.goto(`/environment/${long.id}`);
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
	await page.route('**/api/environment', (route) => route.fulfill({ json: [] }));
	await page.goto('/environment');
	await page.getByRole('button', { name: 'Import .env' }).first().click();
	const dialog = page.getByRole('dialog', { name: 'Import variables' });
	const button = (name: string) => dialog.getByRole('button', { name, exact: true });
	await expect(button('Import')).toBeDisabled();
	await dialog.getByLabel('Variables').fill('EDITOR=vim');
	await expect(button('Import 1 variable')).toBeEnabled();
	await dialog.getByLabel('Variables').fill('EDITOR=vim\nPAGER=less');
	await expect(button('Import 2 variables')).toBeEnabled();
});

test('an entry lives through Corral: made, renamed, made secret and deleted', async ({ page }) => {
	const suffix = Math.random().toString(36).slice(2, 8);
	const first = `e2e_${suffix}`;
	const renamed = `E2E_${suffix.toUpperCase()}`;
	await page.goto('/environment');
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

	await page.getByRole('link', { name: 'Environment' }).first().click();
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
	await expect(page).toHaveURL('/environment');
	await expect(page.getByRole('link', { name: new RegExp(renamed) })).toHaveCount(0);
});
