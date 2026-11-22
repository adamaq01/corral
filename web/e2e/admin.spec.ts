import type { Page } from '@playwright/test';
import { account, expect, signIn, test } from './fixtures';

test.describe('an admin', () => {
	test.use({ role: 'admin' });

	test('runs a local account from creation to deletion', async ({ page, browser }) => {
		const carol = { username: 'carol', password: 'carol, first of her name' };

		await page.goto('/admin/accounts');
		await page.getByRole('button', { name: 'New account' }).click();
		const create = page.getByRole('dialog', { name: 'New local account' });
		await create.getByLabel('Username').fill(carol.username);
		await create.getByLabel('Password').fill(carol.password);
		await expect(create.getByRole('radio', { name: /^Member/ })).toBeChecked();
		await create.getByRole('button', { name: 'Create account' }).click();
		await expect(create).toBeHidden();
		await expect(page.getByRole('status')).toHaveText('Created carol, who can sign in now.');
		const accounts = page.getByRole('table', { name: 'Local accounts' });
		await expect(accounts.getByRole('row', { name: /carol/ })).toContainText('Member');

		// A context made here would otherwise start with the admin's session from `test.use`.
		const context = await browser.newContext({ storageState: { cookies: [], origins: [] } });
		const member = await context.newPage();
		await member.goto('/sign-in');
		await signIn(member, carol);
		await expect(member).toHaveURL('/sandboxes');
		const created = await member.request.post('/api/sandboxes', { data: { name: 'main' } });
		expect(created.status(), await created.text()).toBe(201);

		await page.goto('/admin/sandboxes');
		const sandboxes = page.getByRole('table', { name: 'All sandboxes' });
		await expect(sandboxes.getByRole('row', { name: /carol/ })).toContainText('main');

		await page.goto('/admin/users');
		const users = page.getByRole('table', { name: 'Users' });
		const user = users.getByRole('row', { name: /carol/ });
		await expect(user).toContainText('Active');
		await rowAction(page, 'carol', 'Disable');
		await expect(user).toContainText('Disabled');

		await member.reload();
		await expect(member).toHaveURL(/^[^?]*\/sign-in\?error=/);
		await expect(member.getByRole('alert')).toContainText('this account is disabled');
		const form = member.getByRole('form', { name: 'Local account' });
		await signIn(member, carol);
		await expect(form.getByRole('alert')).toHaveText('this account is disabled');

		await rowAction(page, 'carol', 'Enable');
		await expect(user).toContainText('Active');
		await signIn(member, carol);
		await expect(member).toHaveURL('/sandboxes');

		await page.goto('/admin/accounts');
		await rowAction(page, 'carol', 'Reset password');
		const reset = page.getByRole('dialog', { name: 'Reset the password of carol' });
		const renewed = { ...carol, password: 'carol, second of her name' };
		await reset.getByLabel('New password').fill(renewed.password);
		await reset.getByRole('button', { name: 'Reset password' }).click();
		await expect(reset).toBeHidden();
		await expect(page.getByRole('status')).toHaveText('Reset the password of carol.');

		await context.clearCookies();
		await member.goto('/sign-in');
		await signIn(member, carol);
		await expect(form.getByRole('alert')).toHaveText('wrong username or password');
		await signIn(member, renewed);
		await expect(member).toHaveURL('/sandboxes');

		await rowAction(page, 'carol', 'Delete');
		const confirm = page.getByRole('alertdialog', { name: 'Delete carol?' });
		await expect(confirm).toContainText('sandboxes, SSH keys and credentials');
		await confirm.getByRole('button', { name: 'Delete account' }).click();
		await expect(confirm).toBeHidden();
		await expect(accounts.getByRole('row', { name: /carol/ })).toHaveCount(0);

		for (const path of ['/admin/users', '/admin/sandboxes']) {
			await page.goto(path);
			await loaded(page);
			await expect(page.getByRole('row', { name: /carol/ })).toHaveCount(0);
		}

		await member.reload();
		await expect(member.getByRole('alert')).toContainText('this account was deleted');
		await context.close();
	});

	test('cannot disable or delete themselves from the users page', async ({ page }) => {
		await page.goto('/admin/users');
		const self = page.getByRole('table', { name: 'Users' }).getByRole('row', {
			name: new RegExp(account('admin').username)
		});
		await expect(self).toContainText('You');
		await expect(self.getByRole('button')).toHaveCount(0);
		await self.getByRole('link', { name: 'Profile' }).click();
		await expect(page).toHaveURL('/profile');
	});
});

test.describe('a member', () => {
	test.use({ role: 'member' });

	test('never sees the Admin group and is sent away from admin pages', async ({ page }) => {
		for (const path of ['/admin/users', '/admin/accounts', '/admin/sandboxes']) {
			await page.goto(path);
			await expect(page).toHaveURL('/sandboxes');
			await expect(page.getByRole('navigation', { name: 'Main' })).toBeAttached();
			await expect(page.getByRole('navigation', { name: 'Admin' })).toHaveCount(0);
		}
	});
});

/** Waits for the page's table, or its empty state, to show what the API answered. */
async function loaded(page: Page) {
	await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
	await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
}

async function rowAction(page: Page, handle: string, action: string) {
	await page.getByRole('button', { name: `Actions for ${handle}` }).click();
	await page.getByRole('menuitem', { name: action, exact: true }).click();
}
