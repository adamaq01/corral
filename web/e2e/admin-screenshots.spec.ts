import { join } from 'node:path';
import type { Page } from '@playwright/test';
import type {
	LocalAccount,
	OwnedSandbox,
	SandboxStatus,
	User
} from '../src/lib/api/generated/index.js';
import { expect, test } from './fixtures';

const directory = process.env.CORRAL_SCREENSHOTS;

test.skip(!directory, 'CORRAL_SCREENSHOTS names the directory to take screenshots into');

const widths = { desktop: 1280, phone: 390 };

const days = (n: number) => new Date(Date.UTC(2026, 10, 22 - n, 9, 30)).toISOString();

const users: User[] = [
	user('ada', 'admin', 'active', 'ada@example.com', 40),
	user('bob', 'member', 'active', 'bob.the.builder@example.com', 31),
	user('grace-hopper', 'member', 'disabled', null, 12),
	user('linus', 'member', 'deleting', 'linus@example.com', 3)
];

const accounts: LocalAccount[] = [
	{ username: 'ada', role: 'admin', createdAt: days(40) },
	{ username: 'bob', role: 'member', createdAt: days(31) },
	{ username: 'grace-hopper', role: 'member', createdAt: days(12) }
];

const sandboxes: OwnedSandbox[] = [
	sandbox('ada', 'main', 'dev', 'small', true, { state: 'running' }, 39),
	sandbox('ada', 'scratch', 'dev', 'small', false, { state: 'sleeping' }, 20),
	sandbox('bob', 'main', 'dev', 'small', true, { state: 'starting' }, 1),
	sandbox(
		'grace-hopper',
		'compiler',
		'dev',
		'small',
		true,
		{ state: 'failed', detail: 'the image could not be pulled' },
		10
	),
	sandbox('linus', 'kernel', 'dev', 'small', true, { state: 'deleting' }, 3)
];

/** The API answering `path` with `body`, so each screenshot shows the same rows. */
async function answer(page: Page, path: string, body: unknown, status = 200) {
	await page.route(path, (route) =>
		route.request().method() === 'GET' ? route.fulfill({ status, json: body }) : route.fallback()
	);
}

for (const scheme of ['light', 'dark'] as const) {
	for (const [device, width] of Object.entries(widths)) {
		test.describe(`${scheme}, ${device}`, () => {
			test.use({
				role: 'admin',
				colorScheme: scheme,
				viewport: { width, height: device === 'phone' ? 844 : 800 }
			});

			const shot = async (page: Page, name: string) => {
				await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
				await page.screenshot({
					path: join(directory!, `${name}-${scheme}-${width}.png`),
					animations: 'disabled',
					fullPage: true
				});
			};

			test('users', async ({ page }) => {
				await answer(page, '/api/admin/users', users);
				await page.goto('/admin/users');
				await page.getByRole('row', { name: /linus/ }).waitFor();
				await shot(page, 'admin-users');

				await page.getByRole('button', { name: 'Actions for bob' }).click();
				await page.getByRole('menuitem', { name: 'Delete' }).click();
				await page.getByRole('alertdialog').waitFor();
				await shot(page, 'admin-users-delete');
			});

			test('users, empty', async ({ page }) => {
				await answer(page, '/api/admin/users', []);
				await page.goto('/admin/users');
				await page.getByText('No users yet').waitFor();
				await shot(page, 'admin-users-empty');
			});

			test('local accounts', async ({ page }) => {
				await answer(page, '/auth/local/accounts', accounts);
				await page.goto('/admin/accounts');
				await page.getByRole('row', { name: /grace-hopper/ }).waitFor();
				await shot(page, 'admin-accounts');

				await page.getByRole('button', { name: 'New account' }).click();
				await page.getByRole('dialog').waitFor();
				await shot(page, 'admin-accounts-new');
			});

			test('local accounts, empty', async ({ page }) => {
				await answer(page, '/auth/local/accounts', []);
				await page.goto('/admin/accounts');
				await page.getByText('No local accounts').waitFor();
				await shot(page, 'admin-accounts-empty');
			});

			test('all sandboxes', async ({ page }) => {
				await answer(page, '/api/admin/sandboxes', sandboxes);
				await page.goto('/admin/sandboxes');
				await page.getByRole('row', { name: /kernel/ }).waitFor();
				await shot(page, 'admin-sandboxes');
			});

			test('all sandboxes, empty', async ({ page }) => {
				await answer(page, '/api/admin/sandboxes', []);
				await page.goto('/admin/sandboxes');
				await page.getByText('No sandboxes yet').waitFor();
				await shot(page, 'admin-sandboxes-empty');
			});

			test('all sandboxes, failed', async ({ page }) => {
				await answer(page, '/api/admin/sandboxes', { error: 'internal error' }, 500);
				await page.goto('/admin/sandboxes');
				await page.getByRole('button', { name: 'Retry' }).waitFor();
				await shot(page, 'admin-sandboxes-failed');
			});
		});
	}
}

function user(
	handle: string,
	role: User['role'],
	status: User['status'],
	email: string | null,
	age: number
): User {
	return {
		id: crypto.randomUUID(),
		handle,
		email,
		picture: null,
		role,
		status,
		createdAt: days(age)
	};
}

function sandbox(
	owner: string,
	name: string,
	image: string,
	size: string,
	isDefault: boolean,
	status: SandboxStatus,
	age: number
): OwnedSandbox {
	return {
		owner: { handle: owner, picture: null },
		name,
		image,
		size,
		isDefault,
		ssh: { username: `${owner}+${name}`, host: 'corral.example.com', port: 2222 },
		createdAt: days(age),
		...status
	};
}
