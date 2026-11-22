import { join } from 'node:path';
import type { Page } from '@playwright/test';
import type {
	Catalog,
	Credential,
	Me,
	Sandbox,
	SandboxStatus,
	SshKey
} from '../src/lib/api/generated/index.js';
import { test } from './fixtures';

const directory = process.env.CORRAL_SCREENSHOTS;

test.skip(!directory, 'CORRAL_SCREENSHOTS names the directory to take screenshots into');

const widths = { desktop: 1280, phone: 390 };

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
	maxSandboxes: 8,
	ssh: { host: 'corral.example.lan', port: 48291 }
};

function sandbox(
	name: string,
	status: SandboxStatus,
	extra: Partial<Pick<Sandbox, 'image' | 'size' | 'isDefault'>> = {}
): Sandbox {
	return {
		name,
		image: 'dev',
		size: 'small',
		isDefault: false,
		ssh: { username: `bob+${name}`, host: 'corral.example.lan', port: 48291 },
		createdAt: '2026-11-02T09:30:00Z',
		...status,
		...extra
	};
}

const sandboxes: Sandbox[] = [
	sandbox('main', { state: 'running' }, { isDefault: true }),
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

const credentials: Credential[] = [
	{ name: 'ANTHROPIC_API_KEY' },
	{ name: 'GITHUB_TOKEN' },
	{ name: 'OPENAI_BASE_URL' }
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

for (const scheme of ['light', 'dark'] as const) {
	for (const [device, width] of Object.entries(widths)) {
		test.describe(`${scheme}, ${device}`, () => {
			test.use({
				role: 'member',
				colorScheme: scheme,
				viewport: { width, height: device === 'phone' ? 844 : 800 }
			});

			const shot = (page: Page, name: string, fullPage = true) =>
				page.screenshot({
					path: join(directory!, `${name}-${scheme}-${width}.png`),
					animations: 'disabled',
					fullPage
				});

			test('sandboxes', async ({ page }) => {
				await serve(page, { '/api/sandboxes': sandboxes, '/api/catalog': catalog });
				await page.goto('/sandboxes');
				await page.getByText('the container exited with code 137').waitFor();
				await shot(page, 'sandboxes');

				await page.getByRole('button', { name: 'New sandbox' }).click();
				await page.getByRole('dialog').waitFor();
				await shot(page, 'sandboxes-new', false);
			});

			test('sandboxes, empty, loading and failed', async ({ page }) => {
				await serve(page, { '/api/sandboxes': [], '/api/catalog': catalog });
				await page.goto('/sandboxes');
				await page.getByText('No sandboxes yet').waitFor();
				await shot(page, 'sandboxes-empty');

				await page.unrouteAll();
				await page.route('**/api/sandboxes', () => {});
				await page.reload();
				await page.getByRole('status', { name: 'Loading sandboxes' }).waitFor();
				await shot(page, 'sandboxes-loading');

				await page.unrouteAll();
				await page.route('**/api/sandboxes', (route) =>
					route.fulfill({ status: 500, json: { error: 'internal error' } })
				);
				await page.reload();
				await page.getByText('Loading your sandboxes failed').waitFor();
				await shot(page, 'sandboxes-failed');
			});

			test('a sandbox', async ({ page }) => {
				await serve(page, { '/api/sandboxes/main': sandboxes[0], '/api/catalog': catalog });
				await page.goto('/sandboxes/main');
				await page.getByText('Connect', { exact: true }).waitFor();
				await shot(page, 'sandbox');

				await page.getByRole('button', { name: 'Delete sandbox' }).click();
				await page.getByRole('alertdialog').waitFor();
				await shot(page, 'sandbox-delete', false);
			});

			test('a failed sandbox, and a missing one', async ({ page }) => {
				await serve(page, { '/api/sandboxes/broken': sandboxes[3], '/api/catalog': catalog });
				await page.goto('/sandboxes/broken');
				await page.getByText('The sandbox failed').waitFor();
				await shot(page, 'sandbox-failed');

				await page.goto('/sandboxes/missing');
				await page.getByText('No sandbox named missing').waitFor();
				await shot(page, 'sandbox-missing');
			});

			test('SSH keys', async ({ page }) => {
				await serve(page, { '/api/keys': keys });
				await page.goto('/keys');
				await page.getByText('bob@laptop').waitFor();
				await shot(page, 'keys');

				await page.unrouteAll();
				await page.getByRole('button', { name: 'Add key' }).click();
				const dialog = page.getByRole('dialog');
				await dialog.getByLabel('Public key').fill('ssh-ed25519 not-a-key bob@phone');
				await dialog.getByRole('button', { name: 'Add key' }).click();
				await dialog.getByRole('alert').waitFor();
				await shot(page, 'keys-add', false);
			});

			test('SSH keys, empty', async ({ page }) => {
				await serve(page, { '/api/keys': [] });
				await page.goto('/keys');
				await page.getByText('No SSH keys yet').waitFor();
				await shot(page, 'keys-empty');
			});

			test('credentials', async ({ page }) => {
				await serve(page, { '/api/credentials': credentials });
				await page.goto('/credentials');
				await page.getByText('GITHUB_TOKEN').waitFor();
				await shot(page, 'credentials');

				await page.getByRole('button', { name: 'Add credential' }).click();
				await page.getByRole('dialog').getByLabel('Name').fill('github_token');
				await shot(page, 'credentials-add', false);
			});

			test('credentials, empty', async ({ page }) => {
				await serve(page, { '/api/credentials': [] });
				await page.goto('/credentials');
				await page.getByText('No credentials yet').waitFor();
				await shot(page, 'credentials-empty');
			});

			test('profile', async ({ page }) => {
				await serve(page, { '/api/me': me({ picture }) });
				await page.goto('/profile');
				await page.getByRole('heading', { name: 'bob' }).waitFor();
				await page.waitForFunction(() =>
					[...document.images].every((image) => image.complete && image.naturalWidth > 0)
				);
				await shot(page, 'profile');

				await page.getByRole('button', { name: 'Delete account' }).click();
				await page.getByRole('alertdialog').getByLabel('Type bob to confirm').fill('bo');
				await shot(page, 'profile-delete', false);
			});

			test('profile, with initials', async ({ page }) => {
				await serve(page, { '/api/me': me({ email: null }) });
				await page.goto('/profile');
				await page.getByRole('heading', { name: 'bob' }).waitFor();
				await shot(page, 'profile-initials');
			});
		});
	}
}
