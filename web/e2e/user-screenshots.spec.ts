import type { Page } from '@playwright/test';
import type {
	AttachedEntry,
	Catalog,
	Me,
	Sandbox,
	SandboxStatus,
	SetupEntry,
	SshKey
} from '../src/lib/api/generated/index.js';
import { expect, test, idle } from './fixtures';
import { inEveryView } from './views';

const catalog: Catalog = {
	images: [
		{ key: 'dev', label: 'Development' },
		{ key: 'python', label: 'Python 3.14' }
	],
	sizes: [
		{ key: 'small', label: 'Small, 2 CPUs and 4 GB' },
		{ key: 'large', label: 'Large, 8 CPUs and 16 GB' }
	],
	defaultImage: 'dev',
	defaultSize: 'small',
	defaultsWithdrawn: false,
	maxSandboxes: 8,
	ssh: { host: 'corral.example.lan', port: 48291 }
};

function sandbox(
	name: string,
	status: SandboxStatus,
	extra: Partial<Pick<Sandbox, 'image' | 'size' | 'isDefault' | 'staleSetup' | 'deleteAt'>> = {}
): Sandbox {
	return {
		name,
		image: 'dev',
		size: 'small',
		isDefault: false,
		ssh: { username: `${name}.bob`, host: 'corral.example.lan', port: 48291 },
		createdAt: '2026-11-02T09:30:00Z',
		staleSetup: false,
		idle,
		deleteAt: null,
		repository: null,
		...status,
		...extra
	};
}

const sandboxes: Sandbox[] = [
	sandbox('main', { state: 'running' }, { isDefault: true }),
	sandbox('agents', { state: 'running' }, { staleSetup: true }),
	sandbox('experiments', { state: 'starting' }, { size: 'large' }),
	sandbox('docs', { state: 'sleeping' }, { image: 'python' }),
	sandbox('broken', { state: 'failed', detail: 'the container exited with code 137' }),
	sandbox('old', { state: 'deleting' })
];

const keys: SshKey[] = [
	{
		id: '0b9c6f0e-1f3e-4a4e-9a52-6a1c0f1d2e3f',
		label: 'bob@laptop',
		publicKey: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAm2pcbvM3oNh0KqZf0uQ7xV0b1Wnq1y0x7ZJf7pA0ZB',
		fingerprint: 'SHA256:9kJ1o8m3a1rZpTq2b5YkX7lC0sN4vF6uW8eD3hG2iQk',
		createdAt: '2026-11-02T09:00:00Z'
	},
	{
		id: '6a2d1e47-0c0f-4f6b-8d1e-2c7b9f3a4d5e',
		label: 'Work desktop, the one under the desk with the long name',
		publicKey: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIK3wq0Xv1qk1uYQ8f1h2Lr0mJ7cB9nT4aZ5sD6eF7gH8',
		fingerprint: 'SHA256:Qz3rT8yU1iO5pA7sD9fG2hJ4kL6zX8cV0bN3mQ5wE7r',
		createdAt: '2026-11-20T17:45:00Z'
	}
];

const setAt = '2026-11-20T17:45:00Z';

const entries: SetupEntry[] = [
	{
		id: '1d9e0f1a-2b3c-4d4e-8f5a-6b7c8d9e0f1a',
		kind: 'variable',
		target: 'ANTHROPIC_BASE_URL',
		secret: false,
		template: false,
		applyToNew: true,
		value: { setAt, size: 40, shown: 'text', text: 'https://gateway.example.lan/v1/anthropic' },
		createdAt: setAt,
		sandboxes: sandboxes.map(({ name }) => ({ sandbox: name, overridden: false, unresolved: [] })),
		executable: false,
		references: [],
		duplicate: false,
		shared: 'required',
		shadows: false,
		shadowed: false
	},
	{
		id: '2e0f1a2b-3c4d-4e5f-9a6b-7c8d9e0f1a2b',
		kind: 'variable',
		target: 'HF_TOKEN',
		secret: true,
		template: false,
		applyToNew: false,
		value: { setAt, size: 37, shown: 'hidden' },
		createdAt: setAt,
		sandboxes: [{ sandbox: 'main', overridden: false, unresolved: [] }],
		executable: false,
		references: [],
		duplicate: false,
		shared: 'offered',
		shadows: false,
		shadowed: false
	},
	{
		id: '3f1c2a4e-5b6d-4e7f-8a9b-0c1d2e3f4a5b',
		kind: 'variable',
		target: 'ANTHROPIC_API_KEY',
		secret: true,
		template: false,
		applyToNew: true,
		value: { setAt, size: 108, shown: 'masked', length: 108, head: 'sk-a', tail: 'AA' },
		createdAt: setAt,
		sandboxes: [
			{ sandbox: 'main', overridden: false, unresolved: [] },
			{ sandbox: 'agents', overridden: false, unresolved: [] }
		],
		executable: false,
		references: [],
		duplicate: false,
		shared: null,
		shadows: false,
		shadowed: false
	},
	{
		id: '4a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d',
		kind: 'variable',
		target: 'GITHUB_PIN',
		secret: true,
		template: false,
		applyToNew: false,
		value: { setAt, size: 6, shown: 'masked', length: 6, head: null, tail: null },
		createdAt: setAt,
		sandboxes: [],
		executable: false,
		references: [],
		duplicate: false,
		shared: null,
		shadows: false,
		shadowed: false
	},
	{
		id: '5b3c4d5e-6f7a-4b8c-9d0e-1f2a3b4c5d6e',
		kind: 'variable',
		target: 'OPENAI_BASE_URL',
		secret: false,
		template: false,
		applyToNew: true,
		value: { setAt, size: 33, shown: 'text', text: 'https://gateway.example.lan/v1/openai' },
		createdAt: setAt,
		sandboxes: [{ sandbox: 'main', overridden: false, unresolved: [] }],
		executable: false,
		references: [],
		duplicate: false,
		shared: null,
		shadows: false,
		shadowed: false
	},
	{
		id: '6c4d5e6f-7a8b-4c9d-0e1f-2a3b4c5d6e7f',
		kind: 'file',
		target: '.config/tool/config.toml',
		secret: false,
		template: true,
		applyToNew: true,
		value: {
			setAt,
			size: 77,
			shown: 'text',
			text: '[api]\nurl = "{{ OPENAI_BASE_URL }}"\nkey = "{{ ANTHROPIC_API_KEY }}"\nretries = 3\n'
		},
		createdAt: setAt,
		sandboxes: [{ sandbox: 'main', overridden: false, unresolved: [] }],
		executable: false,
		references: ['ANTHROPIC_API_KEY', 'OPENAI_BASE_URL'],
		duplicate: false,
		shared: null,
		shadows: false,
		shadowed: false
	},
	{
		id: '7d5e6f7a-8b9c-4d0e-1f2a-3b4c5d6e7f8a',
		kind: 'file',
		target: '.ssh/id_ed25519',
		secret: true,
		template: false,
		applyToNew: false,
		value: { setAt, size: 411, shown: 'hidden' },
		createdAt: setAt,
		sandboxes: [{ sandbox: 'agents', overridden: false, unresolved: [] }],
		executable: false,
		references: [],
		duplicate: false,
		shared: null,
		shadows: false,
		shadowed: false
	}
];

const attached: AttachedEntry[] = [
	{ entry: entries[0], override: null },
	{ entry: entries[1], override: null },
	{
		entry: entries[2],
		override: { setAt, size: 51, shown: 'masked', length: 51, head: 'sk-a', tail: 'Qw' }
	},
	{ entry: entries[4], override: null },
	{ entry: entries[5], override: null }
];

const picture = 'https://pictures.example/bob.svg';
const me = (extra: Partial<Me> = {}): Me => ({
	handle: 'bob',
	email: 'bob@example.lan',
	picture: null,
	role: 'member',
	provider: 'local',
	...extra
});

async function serve(page: Page, routes: Record<string, unknown>) {
	for (const [path, body] of Object.entries(routes)) {
		await page.route(`**${path}`, (route) => route.fulfill({ json: body }));
	}
	await page.route(picture, (route) =>
		route.fulfill({
			contentType: 'image/svg+xml',
			body: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8"><rect width="8" height="8" fill="#2a9d8f"/><circle cx="4" cy="3" r="1.6" fill="#fff"/><rect x="1.5" y="5.2" width="5" height="3" rx="1.5" fill="#fff"/></svg>'
		})
	);
}

inEveryView((look) => {
	test.use({ role: 'member' });

	test('sandboxes', async ({ page }) => {
		await serve(page, {
			'/api/sandboxes': sandboxes,
			'/api/catalog': catalog,
			'/api/setup': entries
		});
		await page.goto('/sandboxes');
		await page.getByText('the container exited with code 137').waitFor();
		await look(page, 'sandboxes');

		await page.getByRole('button', { name: 'New sandbox' }).click();
		const dialog = page.getByRole('dialog');
		await dialog.getByRole('list', { name: 'What it comes with' }).waitFor();
		await expect(dialog.getByRole('group', { name: 'Repository' })).toBeVisible();
		await look(page, 'sandboxes-new', { fullPage: false });
	});

	test('sandboxes, empty, loading and failed', async ({ page }) => {
		await serve(page, { '/api/sandboxes': [], '/api/catalog': catalog, '/api/setup': [] });
		await page.goto('/sandboxes');
		await page.getByText('No sandboxes yet').waitFor();
		await look(page, 'sandboxes-empty');

		await page.unrouteAll();
		await page.route('**/api/sandboxes', () => {});
		await page.reload();
		await page.getByRole('status', { name: 'Loading sandboxes' }).waitFor();
		await look(page, 'sandboxes-loading');

		await page.unrouteAll();
		await page.route('**/api/sandboxes', (route) =>
			route.fulfill({ status: 500, json: { error: 'internal error' } })
		);
		await page.reload();
		await page.getByText('Loading your sandboxes failed').waitFor();
		await look(page, 'sandboxes-failed');
	});

	test('a sandbox', async ({ page }) => {
		await serve(page, {
			'/api/sandboxes/main': sandboxes[0],
			'/api/sandboxes/main/setup': attached,
			'/api/setup': entries,
			'/api/catalog': catalog
		});
		await page.goto('/sandboxes/main');
		await page.getByText('Connect', { exact: true }).waitFor();
		await page.getByRole('list', { name: 'Attached to main' }).waitFor();
		await look(page, 'sandbox');

		await page.getByRole('button', { name: 'Delete sandbox' }).click();
		await page.getByRole('alertdialog').waitFor();
		await look(page, 'sandbox-delete', { fullPage: false });
	});

	test('a sandbox scheduled for deletion', async ({ page }) => {
		const doomed = sandbox('docs', { state: 'sleeping' }, { deleteAt: '2026-11-24T18:40:00Z' });
		await serve(page, { '/api/sandboxes/docs': doomed, '/api/catalog': catalog });
		await page.goto('/sandboxes/docs');
		const alert = page.getByRole('alert').filter({ hasText: 'Deletion scheduled for' });
		await alert.waitFor();
		const box = await alert.boundingBox();
		const button = await alert.getByRole('button', { name: 'Cancel deletion' }).boundingBox();
		expect(box && button).toBeTruthy();
		expect(button!.x + button!.width).toBeLessThanOrEqual(box!.x + box!.width);
		expect(button!.y + button!.height).toBeLessThanOrEqual(box!.y + box!.height);
		await look(page, 'sandbox-deletion-scheduled');
	});

	test('a page that does not exist', async ({ page }) => {
		await page.goto('/nowhere/at/all');
		await page.getByText('There is no such page').waitFor();
		await look(page, 'not-found');
	});

	test('a page Corral cannot answer for', async ({ page }) => {
		await page.route('**/api/me', (route) => route.abort());
		await page.goto('/sandboxes');
		await page.getByText('Corral could not load this page').waitFor();
		await expect(page.getByText('Corral cannot be reached')).toBeVisible();
		await look(page, 'unreachable');
	});

	test('a sandbox missing a setup change', async ({ page }) => {
		await serve(page, {
			'/api/sandboxes/agents': sandboxes[1],
			'/api/sandboxes/agents/setup': [{ entry: entries[0], override: null }],
			'/api/setup': entries,
			'/api/catalog': catalog
		});
		await page.goto('/sandboxes/agents');
		await page.getByText('Restart to apply your setup changes').waitFor();
		await look(page, 'sandbox-stale-setup');
	});

	test("a sandbox's setup", async ({ page }) => {
		await serve(page, {
			'/api/sandboxes/main': sandboxes[0],
			'/api/sandboxes/main/setup': attached,
			'/api/setup': entries,
			'/api/catalog': catalog
		});
		await page.goto('/sandboxes/main');
		await page
			.getByRole('list', { name: 'Attached to main' })
			.getByText('Override', { exact: true })
			.waitFor();
		await look(page, 'sandbox-setup');

		await page.getByRole('button', { name: 'Attach', exact: true }).click();
		await page.getByRole('dialog').waitFor();
		await look(page, 'sandbox-setup-attach', { fullPage: false });

		await page.keyboard.press('Escape');
		await page.getByRole('dialog').waitFor({ state: 'hidden' });
		await page.getByRole('button', { name: 'Change override of ANTHROPIC_API_KEY' }).click();
		await page.getByRole('dialog').getByLabel('Value', { exact: true }).fill('sk-ant-main');
		await look(page, 'sandbox-setup-override', { fullPage: false });
	});

	test('a failed sandbox, and a missing one', async ({ page }) => {
		await serve(page, { '/api/sandboxes/broken': sandboxes[4], '/api/catalog': catalog });
		await page.goto('/sandboxes/broken');
		await page.getByText('The sandbox failed').waitFor();
		await look(page, 'sandbox-failed');

		await page.goto('/sandboxes/missing');
		await page.getByText('No sandbox named missing').waitFor();
		await look(page, 'sandbox-missing');
	});

	test('setup', async ({ page }) => {
		await serve(page, { '/api/setup': entries });
		await page.goto('/setup');
		await page.getByText('ANTHROPIC_API_KEY').waitFor();
		await expect(page.getByRole('list', { name: 'Presets' })).toContainText(
			'Anthropic through a gateway'
		);
		await look(page, 'setup');

		await page.getByRole('button', { name: 'Change LLM providers' }).click();
		const dialog = page.getByRole('dialog');
		await dialog.getByText('Type the key again so it is also sent as a bearer token.').waitFor();
		await look(page, 'setup-llm', { fullPage: false });

		await page.keyboard.press('Escape');
		await dialog.waitFor({ state: 'hidden' });
		await page.getByRole('button', { name: 'Change Git identity' }).click();
		await dialog.getByLabel('Name', { exact: true }).fill('Bob Example');
		await dialog.getByLabel('Email', { exact: true }).fill('bob@example.lan');
		await expect(dialog.getByRole('radio', { name: 'An SSH key' })).toBeChecked();
		await expect(dialog).toContainText('Set, hidden');
		await look(page, 'setup-git', { fullPage: false });

		await page.keyboard.press('Escape');
		await dialog.waitFor({ state: 'hidden' });
		await page.getByRole('button', { name: 'Set up MCP servers' }).click();
		const fetch = dialog.getByRole('group', { name: 'Server 1' });
		await fetch.getByLabel('Name', { exact: true }).fill('fetch');
		await fetch.getByLabel('Command', { exact: true }).fill('uvx');
		await fetch.getByLabel('Arguments', { exact: true }).fill('mcp-server-fetch');
		await dialog.getByRole('button', { name: 'Add server' }).click();
		const docs = dialog.getByRole('group', { name: 'Server 2' });
		await docs.getByLabel('Name', { exact: true }).fill('docs');
		await docs.getByRole('radio', { name: 'A URL' }).click();
		await docs.getByLabel('URL', { exact: true }).fill('https://docs.example.lan/mcp');
		await docs.getByLabel('Headers', { exact: true }).fill('X-Team: platform');
		await look(page, 'setup-mcp', { fullPage: false });

		await page.keyboard.press('Escape');
		await dialog.waitFor({ state: 'hidden' });
		await page.getByRole('button', { name: 'Add variable' }).click();
		await dialog.getByLabel('Name').fill('github_token');
		await dialog.getByRole('switch', { name: 'Secret' }).click();
		await dialog.getByLabel('Value', { exact: true }).fill('ghp_example0123456789');
		await dialog.getByRole('button', { name: 'Show the value' }).click();
		await look(page, 'setup-add-variable', { fullPage: false });

		await page.keyboard.press('Escape');
		await dialog.waitFor({ state: 'hidden' });
		await page.getByRole('button', { name: 'Add file' }).click();
		await dialog.getByLabel('Path in the home').fill('.config/gh/hosts.yml');
		await dialog
			.getByLabel('Value', { exact: true })
			.fill('github.com:\n  oauth_token: {{ GH_TOKEN }}\n');
		await dialog.getByRole('switch', { name: 'Template' }).click();
		await look(page, 'setup-add-file', { fullPage: false });

		await page.keyboard.press('Escape');
		await dialog.waitFor({ state: 'hidden' });
		await page.getByRole('button', { name: 'Import .env' }).click();
		await dialog
			.getByLabel('Variables')
			.fill(
				'# from the laptop\nexport OPENAI_BASE_URL=https://gateway.example.lan/v1\nHF_TOKEN="hf_example"\nnot a line'
			);
		await look(page, 'setup-import', { fullPage: false });
	});

	test('setup, empty, loading and failed', async ({ page }) => {
		await serve(page, { '/api/setup': [] });
		await page.goto('/setup');
		await page.getByText('No variables or files yet').waitFor();
		await look(page, 'setup-empty');

		await page.unrouteAll();
		await page.route('**/api/setup', () => {});
		await page.reload();
		await page.getByRole('status', { name: 'Loading your setup' }).waitFor();
		await look(page, 'setup-loading');

		await page.unrouteAll();
		await page.route('**/api/setup', (route) =>
			route.fulfill({ status: 500, json: { error: 'internal error' } })
		);
		await page.reload();
		await page.getByText('Loading your setup failed').waitFor();
		await look(page, 'setup-failed');
	});

	test('a setup entry', async ({ page }) => {
		for (const [name, entry] of [
			['setup-secret-variable', entries[2]],
			['setup-template-file', entries[5]],
			['setup-secret-file', entries[6]]
		] as const) {
			await serve(page, {
				[`/api/setup/${entry.id}`]: entry,
				'/api/sandboxes': sandboxes,
				'/api/sandboxes/main/setup/*/preview': {
					text: '[api]\nurl = "https://gateway.example.lan/v1/openai"\nkey = "••••••••"\nretries = 3\n',
					unresolved: []
				}
			});
			await page.goto(`/setup/${entry.id}`);
			await page.getByRole('list', { name: 'Your sandboxes' }).waitFor();
			if (entry.template) {
				await page.getByRole('button', { name: 'Preview' }).click();
				await page.getByText('secret values masked').waitFor();
			}
			await look(page, name);
		}

		await page.getByRole('button', { name: 'Replace value' }).click();
		await page.getByRole('dialog').waitFor();
		await look(page, 'setup-replace', { fullPage: false });

		await page.keyboard.press('Escape');
		await page.getByRole('dialog').waitFor({ state: 'hidden' });
		await page.getByRole('button', { name: 'Delete file' }).click();
		await page.getByRole('alertdialog').waitFor();
		await look(page, 'setup-delete', { fullPage: false });

		await page.goto('/setup/00000000-0000-0000-0000-000000000000');
		await page.getByText('No such variable or file').waitFor();
		await look(page, 'setup-missing');
	});

	test('a shared setup entry', async ({ page }) => {
		const [required] = entries;
		await serve(page, { [`/api/setup/${required.id}`]: required, '/api/sandboxes': sandboxes });
		await page.goto(`/setup/${required.id}`);
		await page.getByText('Set by the admin in Settings > Shared setup').waitFor();
		await look(page, 'setup-shared-entry');
	});

	test('account', async ({ page }) => {
		await serve(page, { '/api/me': me({ picture }), '/api/catalog': catalog, '/api/keys': keys });
		await page.goto('/account');
		await page.getByRole('heading', { name: 'bob' }).waitFor();
		await page.getByText('bob@laptop').waitFor();
		await page.waitForFunction(() =>
			[...document.images].every((image) => image.complete && image.naturalWidth > 0)
		);
		await look(page, 'account');

		await page.getByRole('button', { name: 'Change password' }).click();
		await page.getByRole('dialog', { name: 'Change your password' }).waitFor();
		await look(page, 'account-password', { fullPage: false });

		await page.keyboard.press('Escape');
		await page.getByRole('dialog').waitFor({ state: 'hidden' });
		await page.getByRole('button', { name: 'Delete account' }).click();
		await page.getByRole('alertdialog').getByLabel('Type bob to confirm').fill('bo');
		await look(page, 'account-delete', { fullPage: false });

		await page.keyboard.press('Escape');
		await page.getByRole('alertdialog').waitFor({ state: 'hidden' });
		await page.unrouteAll();
		await page.getByRole('button', { name: 'Add key' }).click();
		const dialog = page.getByRole('dialog');
		await dialog.getByLabel('Public key').fill('ssh-ed25519 not-a-key bob@phone');
		await dialog.getByRole('button', { name: 'Add key' }).click();
		await dialog.getByRole('alert').waitFor();
		await look(page, 'account-add-key', { fullPage: false });
	});

	test('account, with unsaved defaults, one no longer offered', async ({ page }) => {
		await serve(page, {
			'/api/me': me(),
			'/api/catalog': { ...catalog, defaultsWithdrawn: true },
			'/api/keys': keys
		});
		await page.goto('/account');
		await page.getByText('Your saved default is no longer offered').waitFor();
		await page.getByLabel('Image').selectOption('python');
		await page.getByText('Unsaved changes').waitFor();
		await look(page, 'account-unsaved');
	});

	test('account, with initials and no SSH keys', async ({ page }) => {
		await serve(page, {
			'/api/me': me({ email: null, provider: 'sso' }),
			'/api/catalog': catalog,
			'/api/keys': []
		});
		await page.goto('/account');
		await page.getByRole('heading', { name: 'bob' }).waitFor();
		await page.getByText('No SSH keys yet').waitFor();
		await expect(page.getByRole('button', { name: 'Change password' })).toHaveCount(0);
		await look(page, 'account-initials');
	});
});
