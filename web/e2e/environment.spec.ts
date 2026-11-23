import type { Page, Route } from '@playwright/test';
import type { AttachedEntry, EnvironmentEntry, Sandbox } from '../src/lib/api/generated/index.js';
import { expect, test } from './fixtures';

test.use({ role: 'member' });

const setAt = '2026-11-20T17:45:00Z';

function entry(extra: Partial<EnvironmentEntry>): EnvironmentEntry {
	return {
		id: '3f1c2a4e-5b6d-4e7f-8a9b-0c1d2e3f4a5b',
		kind: 'variable',
		target: 'GH_TOKEN',
		secret: true,
		template: false,
		applyToNew: true,
		value: { setAt, size: 40, shown: 'masked', length: 40, head: 'ghp_', tail: 'Xz' },
		createdAt: setAt,
		sandboxes: [],
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
	await page.goto('/environment');
	await page.getByRole('button', { name: 'Add variable' }).first().click();
	const dialog = page.getByRole('dialog', { name: 'Add a variable' });
	const value = dialog.getByLabel('Value', { exact: true });

	await dialog.getByLabel('Name').fill('gh_token');
	await expect(dialog.getByLabel('Name')).toHaveValue('GH_TOKEN');
	await expect(dialog.getByRole('button', { name: 'Show the value' })).toHaveCount(0);
	await dialog.getByLabel('Secret').check();
	const show = dialog.getByRole('button', { name: 'Show the value' });
	await value.fill('ghp_typed');
	await expect(value).toHaveAttribute('type', 'password');
	await show.click();
	await expect(value).toHaveAttribute('type', 'text');
	await expect(show).toHaveAttribute('aria-pressed', 'true');
	await value.press('End');
	await value.pressSequentially('-more');
	await expect(value).toHaveValue('ghp_typed-more');
	await dialog.getByLabel('Apply to new sandboxes').uncheck();
	await dialog.getByRole('button', { name: 'Save variable' }).click();

	await expect(page).toHaveURL(`/environment/${entry({}).id}`);
	await expect(page.getByText('ghp_…Xz, 40 characters')).toBeVisible();
	expect(calls.map((call) => call.body)).toEqual([
		{
			kind: 'variable',
			target: 'GH_TOKEN',
			secret: true,
			template: false,
			applyToNew: false,
			value: { text: 'ghp_typed-more' }
		}
	]);
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
	await dialog.getByLabel('Path in the home').fill('~/.config/tool.toml');
	await dialog.getByLabel('Value', { exact: true }).fill('a = 1\nb = "{{ GH_TOKEN }}"\n');
	await dialog.getByLabel('Template').check();
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
	await dialog.getByLabel('Template').uncheck();
	await dialog.getByLabel('Secret').check();
	await dialog.getByRole('button', { name: 'Save file' }).click();
	await expect(dialog.getByRole('alert')).toBeVisible();

	expect(sent).toEqual([
		{
			kind: 'file',
			target: '.config/tool.toml',
			secret: false,
			template: true,
			applyToNew: true,
			value: { text: 'a = 1\nb = "{{ GH_TOKEN }}"\n' }
		},
		{
			kind: 'file',
			target: '.config/tool.toml',
			secret: true,
			template: false,
			applyToNew: true,
			value: { base64: 'AJ+Slv8=' }
		}
	]);
});

test("an entry's settings save as they change", async ({ page }) => {
	const file = entry({
		id: '6c4d5e6f-7a8b-4c9d-0e1f-2a3b4c5d6e7f',
		kind: 'file',
		target: 'notes',
		secret: false,
		value: { setAt, size: 2, shown: 'text', text: 'hi' },
		sandboxes: ['main']
	});
	const calls: Call[] = [];
	await record(
		page,
		`/api/environment/${file.id}`,
		(route) => {
			if (route.request().method() === 'GET') {
				return { json: file };
			}
			const change = route.request().postDataJSON();
			return { json: { ...file, ...change, value: file.value } };
		},
		calls
	);
	await page.goto(`/environment/${file.id}`);
	await expect(page.getByRole('link', { name: 'main' })).toBeVisible();
	await page.getByLabel('Template').check();
	await expect(page.getByText('Saved', { exact: true })).toBeVisible();
	await page.getByLabel('Apply to new sandboxes').uncheck();
	await page.getByRole('button', { name: 'Edit value' }).click();
	const dialog = page.getByRole('dialog', { name: 'Edit ~/notes' });
	await expect(dialog.getByLabel('Value', { exact: true })).toHaveValue('hi');
	await dialog.getByLabel('Value', { exact: true }).fill('hello');
	await dialog.getByRole('button', { name: 'Save value' }).click();
	await expect(dialog).toBeHidden();

	expect(calls.filter((call) => call.method === 'PUT').map((call) => call.body)).toEqual([
		{ template: true, applyToNew: true },
		{ template: true, applyToNew: false },
		{ template: true, applyToNew: false, value: { text: 'hello' } }
	]);
});

test('a sandbox attaches, overrides and detaches entries', async ({ page }) => {
	const token = entry({ sandboxes: ['main'] });
	const notes = entry({
		id: '6c4d5e6f-7a8b-4c9d-0e1f-2a3b4c5d6e7f',
		kind: 'file',
		target: 'notes',
		secret: false,
		value: { setAt, size: 2, shown: 'text', text: 'hi' }
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
		deleteAt: null,
		state: 'running'
	};
	await page.route('**/api/sandboxes/main', (route) => route.fulfill({ json: sandbox }));
	await page.route('**/api/environment', (route) => route.fulfill({ json: [token, notes] }));
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
				attached = [...attached, { entry: notes, override: null }];
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
	await expect(dialog.getByRole('listitem')).toHaveCount(1);
	await dialog.getByRole('button', { name: 'Attach ~/notes' }).click();
	await expect(dialog).toBeHidden();
	await expect(section.getByRole('listitem')).toHaveCount(2);

	await page.getByRole('button', { name: 'Set a value here for GH_TOKEN' }).click();
	const own = page.getByRole('dialog', { name: 'GH_TOKEN in main' });
	await own.getByLabel('Value', { exact: true }).fill('ghp_main');
	await own.getByRole('button', { name: 'Save value' }).click();
	await expect(section.getByText("This sandbox's value")).toBeVisible();
	await expect(section).toContainText('8 characters');
	await page.getByRole('button', { name: 'Use the shared value for GH_TOKEN' }).click();
	await expect(section.getByText("This sandbox's value")).toBeHidden();
	await page.getByRole('button', { name: 'Detach ~/notes' }).click();
	await expect(section.getByRole('listitem')).toHaveCount(1);

	expect(calls.map(({ method, path, body }) => [method, path.split('/').slice(5), body])).toEqual([
		['PUT', [notes.id], null],
		['PUT', [token.id, 'override'], { text: 'ghp_main' }],
		['DELETE', [token.id, 'override'], null],
		['DELETE', [notes.id], null]
	]);
});
