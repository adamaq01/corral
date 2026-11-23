import type { Page } from '@playwright/test';
import type {
	LocalAccount,
	OwnedSandbox,
	SandboxStatus,
	User
} from '../src/lib/api/generated/index.js';
import { expect, test } from './fixtures';
import { inEveryView } from './views';

const days = (n: number) => new Date(Date.UTC(2026, 10, 22 - n, 9, 30)).toISOString();

const users: User[] = [
	user('ada', 'admin', 'active', 'ada@example.com', 40),
	user('bob', 'member', 'active', 'bob.the.builder@example.com', 31),
	user('grace-hopper', 'member', 'disabled', null, 12),
	user('linus', 'member', 'deleting', 'linus@example.com', 3)
];

const accounts: LocalAccount[] = [
	{ username: 'ada', role: 'admin', createdAt: days(40), yours: true },
	{ username: 'bob', role: 'member', createdAt: days(31), yours: false },
	{ username: 'grace-hopper', role: 'member', createdAt: days(12), yours: false }
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

inEveryView((look) => {
	test.use({ role: 'admin' });

	test('users', async ({ page }) => {
		await answer(page, '/api/admin/users', users);
		await page.goto('/admin/users');
		await page.getByRole('row', { name: /linus/ }).waitFor();
		await look(page, 'admin-users');

		await page.getByRole('button', { name: 'Actions for bob' }).click();
		await page.getByRole('menuitem', { name: 'Delete' }).click();
		await page.getByRole('alertdialog').waitFor();
		await look(page, 'admin-users-delete');
	});

	test('users, empty', async ({ page }) => {
		await answer(page, '/api/admin/users', []);
		await page.goto('/admin/users');
		await page.getByText('No users yet').waitFor();
		await look(page, 'admin-users-empty');
	});

	test('local accounts', async ({ page }) => {
		await answer(page, '/auth/local/accounts', accounts);
		await page.goto('/admin/accounts');
		await page.getByRole('row', { name: /grace-hopper/ }).waitFor();
		await look(page, 'admin-accounts');

		await page.getByRole('button', { name: 'New account' }).click();
		await page.getByRole('dialog').waitFor();
		await look(page, 'admin-accounts-new');
	});

	test('local accounts, empty', async ({ page }) => {
		await answer(page, '/auth/local/accounts', []);
		await page.goto('/admin/accounts');
		await page.getByText('No local accounts').waitFor();
		await look(page, 'admin-accounts-empty');
	});

	test('all sandboxes', async ({ page }) => {
		await answer(page, '/api/admin/sandboxes', sandboxes);
		await page.goto('/admin/sandboxes');
		await page.getByRole('row', { name: /kernel/ }).waitFor();
		const detail = page.getByText('the image could not be pulled');
		await expect(detail).toBeVisible();
		expect(await detail.evaluate((text) => text.scrollWidth <= text.clientWidth)).toBe(true);
		await look(page, 'admin-sandboxes');
	});

	test('all sandboxes, empty', async ({ page }) => {
		await answer(page, '/api/admin/sandboxes', []);
		await page.goto('/admin/sandboxes');
		await page.getByText('No sandboxes yet').waitFor();
		await look(page, 'admin-sandboxes-empty');
	});

	test('all sandboxes, failed', async ({ page }) => {
		await answer(page, '/api/admin/sandboxes', { error: 'internal error' }, 500);
		await page.goto('/admin/sandboxes');
		await page.getByRole('button', { name: 'Retry' }).waitFor();
		await look(page, 'admin-sandboxes-failed');
	});
});

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
		staleCredentials: false,
		...status
	};
}
