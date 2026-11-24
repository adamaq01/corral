import type { Page } from '@playwright/test';
import type {
	AttachedEntry,
	Catalog,
	EnvironmentEntry,
	Me,
	Sandbox,
	SandboxStatus,
	SshKey
} from '../src/lib/api/generated/index.js';
import { test } from './fixtures';
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
	extra: Partial<Pick<Sandbox, 'image' | 'size' | 'isDefault' | 'staleEnvironment'>> = {}
): Sandbox {
	return {
		name,
		image: 'dev',
		size: 'small',
		isDefault: false,
		ssh: { username: `${name}.bob`, host: 'corral.example.lan', port: 48291 },
		createdAt: '2026-11-02T09:30:00Z',
		staleEnvironment: false,
		deleteAt: null,
		...status,
		...extra
	};
}

const sandboxes: Sandbox[] = [
	sandbox('main', { state: 'running' }, { isDefault: true }),
	sandbox('agents', { state: 'running' }, { staleEnvironment: true }),
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

const entries: EnvironmentEntry[] = [
	{
		id: '3f1c2a4e-5b6d-4e7f-8a9b-0c1d2e3f4a5b',
		kind: 'variable',
		target: 'ANTHROPIC_API_KEY',
		secret: true,
		template: false,
		applyToNew: true,
		value: { setAt, size: 108, shown: 'masked', length: 108, head: 'sk-a', tail: 'AA' },
		createdAt: setAt,
		sandboxes: ['main', 'agents']
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
		sandboxes: []
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
		sandboxes: ['main']
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
		sandboxes: ['main']
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
		sandboxes: ['agents']
	}
];

const attached: AttachedEntry[] = [
	{
		entry: entries[0],
		override: { setAt, size: 51, shown: 'masked', length: 51, head: 'sk-a', tail: 'Qw' }
	},
	{ entry: entries[2], override: null },
	{ entry: entries[3], override: null }
];

const picture = 'https://pictures.example/bob.svg';
const me = (extra: Partial<Me> = {}): Me => ({
	handle: 'bob',
	email: 'bob@example.lan',
	picture: null,
	role: 'member',
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
		await serve(page, { '/api/sandboxes': sandboxes, '/api/catalog': catalog });
		await page.goto('/sandboxes');
		await page.getByText('the container exited with code 137').waitFor();
		await look(page, 'sandboxes');

		await page.getByRole('button', { name: 'New sandbox' }).click();
		await page.getByRole('dialog').waitFor();
		await look(page, 'sandboxes-new', { fullPage: false });
	});

	test('sandboxes, empty, loading and failed', async ({ page }) => {
		await serve(page, { '/api/sandboxes': [], '/api/catalog': catalog });
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
		await serve(page, { '/api/sandboxes/main': sandboxes[0], '/api/catalog': catalog });
		await page.goto('/sandboxes/main');
		await page.getByText('Connect', { exact: true }).waitFor();
		await look(page, 'sandbox');

		await page.getByRole('button', { name: 'Delete sandbox' }).click();
		await page.getByRole('alertdialog').waitFor();
		await look(page, 'sandbox-delete', { fullPage: false });
	});

	test('a sandbox missing an environment change', async ({ page }) => {
		await serve(page, {
			'/api/sandboxes/agents': sandboxes[1],
			'/api/sandboxes/agents/environment': [],
			'/api/environment': entries,
			'/api/catalog': catalog
		});
		await page.goto('/sandboxes/agents');
		await page.getByText('Restart to apply your environment changes').waitFor();
		await look(page, 'sandbox-stale-environment');
	});

	test("a sandbox's environment", async ({ page }) => {
		await serve(page, {
			'/api/sandboxes/main': sandboxes[0],
			'/api/sandboxes/main/environment': attached,
			'/api/environment': entries,
			'/api/catalog': catalog
		});
		await page.goto('/sandboxes/main');
		await page.getByText("This sandbox's value").waitFor();
		await look(page, 'sandbox-environment');

		await page.getByRole('button', { name: 'Attach', exact: true }).click();
		await page.getByRole('dialog').waitFor();
		await look(page, 'sandbox-environment-attach', { fullPage: false });

		await page.keyboard.press('Escape');
		await page.getByRole('dialog').waitFor({ state: 'hidden' });
		await page.getByRole('button', { name: 'Change for ANTHROPIC_API_KEY' }).click();
		await page.getByRole('dialog').getByLabel('Value', { exact: true }).fill('sk-ant-main');
		await look(page, 'sandbox-environment-override', { fullPage: false });
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

	test('SSH keys', async ({ page }) => {
		await serve(page, { '/api/keys': keys });
		await page.goto('/keys');
		await page.getByText('bob@laptop').waitFor();
		await look(page, 'keys');

		await page.unrouteAll();
		await page.getByRole('button', { name: 'Add key' }).click();
		const dialog = page.getByRole('dialog');
		await dialog.getByLabel('Public key').fill('ssh-ed25519 not-a-key bob@phone');
		await dialog.getByRole('button', { name: 'Add key' }).click();
		await dialog.getByRole('alert').waitFor();
		await look(page, 'keys-add', { fullPage: false });
	});

	test('SSH keys, empty', async ({ page }) => {
		await serve(page, { '/api/keys': [] });
		await page.goto('/keys');
		await page.getByText('No SSH keys yet').waitFor();
		await look(page, 'keys-empty');
	});

	test('environment', async ({ page }) => {
		await serve(page, { '/api/environment': entries });
		await page.goto('/environment');
		await page.getByText('ANTHROPIC_API_KEY').waitFor();
		await look(page, 'environment');

		await page.getByRole('button', { name: 'Add variable' }).click();
		const dialog = page.getByRole('dialog');
		await dialog.getByLabel('Name').fill('github_token');
		await dialog.getByLabel('Secret').check();
		await dialog.getByLabel('Value', { exact: true }).fill('ghp_example0123456789');
		await dialog.getByRole('button', { name: 'Show the value' }).click();
		await look(page, 'environment-add-variable', { fullPage: false });

		await page.keyboard.press('Escape');
		await dialog.waitFor({ state: 'hidden' });
		await page.getByRole('button', { name: 'Add file' }).click();
		await dialog.getByLabel('Path in the home').fill('.config/gh/hosts.yml');
		await dialog
			.getByLabel('Value', { exact: true })
			.fill('github.com:\n  oauth_token: {{ GH_TOKEN }}\n');
		await dialog.getByLabel('Template').check();
		await look(page, 'environment-add-file', { fullPage: false });
	});

	test('environment, empty, loading and failed', async ({ page }) => {
		await serve(page, { '/api/environment': [] });
		await page.goto('/environment');
		await page.getByText('No variables or files yet').waitFor();
		await look(page, 'environment-empty');

		await page.unrouteAll();
		await page.route('**/api/environment', () => {});
		await page.reload();
		await page.getByRole('status', { name: 'Loading your environment' }).waitFor();
		await look(page, 'environment-loading');

		await page.unrouteAll();
		await page.route('**/api/environment', (route) =>
			route.fulfill({ status: 500, json: { error: 'internal error' } })
		);
		await page.reload();
		await page.getByText('Loading your environment failed').waitFor();
		await look(page, 'environment-failed');
	});

	test('an environment entry', async ({ page }) => {
		for (const [name, entry] of [
			['environment-secret-variable', entries[0]],
			['environment-template-file', entries[3]],
			['environment-secret-file', entries[4]]
		] as const) {
			await serve(page, { [`/api/environment/${entry.id}`]: entry });
			await page.goto(`/environment/${entry.id}`);
			await page.getByText('Saved as soon as you change them.').waitFor();
			await look(page, name);
		}

		await page.getByRole('button', { name: 'Replace value' }).click();
		await page.getByRole('dialog').waitFor();
		await look(page, 'environment-replace', { fullPage: false });

		await page.keyboard.press('Escape');
		await page.getByRole('dialog').waitFor({ state: 'hidden' });
		await page.getByRole('button', { name: 'Delete file' }).click();
		await page.getByRole('alertdialog').waitFor();
		await look(page, 'environment-delete', { fullPage: false });

		await page.goto('/environment/00000000-0000-0000-0000-000000000000');
		await page.getByText('No such variable or file').waitFor();
		await look(page, 'environment-missing');
	});

	test('profile', async ({ page }) => {
		await serve(page, { '/api/me': me({ picture }), '/api/catalog': catalog });
		await page.goto('/profile');
		await page.getByRole('heading', { name: 'bob' }).waitFor();
		await page.waitForFunction(() =>
			[...document.images].every((image) => image.complete && image.naturalWidth > 0)
		);
		await look(page, 'profile');

		await page.getByRole('button', { name: 'Delete account' }).click();
		await page.getByRole('alertdialog').getByLabel('Type bob to confirm').fill('bo');
		await look(page, 'profile-delete', { fullPage: false });
	});

	test('profile, with initials', async ({ page }) => {
		await serve(page, { '/api/me': me({ email: null }), '/api/catalog': catalog });
		await page.goto('/profile');
		await page.getByRole('heading', { name: 'bob' }).waitFor();
		await look(page, 'profile-initials');
	});
});
