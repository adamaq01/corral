import type { Locator, Page } from '@playwright/test';
import { account, expect, signIn, test } from './fixtures';

test.describe('an admin', () => {
	test.use({ role: 'admin' });

	test('runs a local account from creation to deletion', async ({ page, browser }) => {
		const carol = { username: 'carol', password: 'carol, first of her name' };

		await page.goto('/admin/people');
		await page.getByRole('button', { name: 'New account' }).click();
		const create = page.getByRole('dialog', { name: 'New local account' });
		await create.getByLabel('Username').fill(carol.username);
		await create.getByLabel('Password', { exact: true }).fill(carol.password);
		await expect(create.getByRole('radio', { name: /^Member/ })).toBeChecked();
		await create.getByRole('button', { name: 'Create account' }).click();
		await expect(create).toBeHidden();
		await expect(page.getByText('Created carol, who can sign in now')).toBeVisible();
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

		await page.goto('/admin/people');
		const users = page.getByRole('table', { name: 'Users' });
		const user = users.getByRole('row', { name: /carol/ });
		await expect(user).toContainText('Active');
		await rowAction(users, 'carol', 'Disable user');
		const disable = page.getByRole('alertdialog', { name: 'Disable carol?' });
		await expect(disable).toContainText('Their sandboxes, SSH keys and environment stay');
		await disable.getByRole('button', { name: 'Disable user' }).click();
		await expect(disable).toBeHidden();
		await expect(user).toContainText('Disabled');

		await member.reload();
		await expect(member).toHaveURL(/^[^?]*\/sign-in\?error=/);
		await expect(member.getByRole('alert')).toContainText('this account is disabled');
		const form = member.getByRole('form', { name: 'Local account' });
		await signIn(member, carol);
		await expect(form.getByRole('alert')).toHaveText('this account is disabled');

		await rowAction(users, 'carol', 'Enable user');
		await expect(user).toContainText('Active');
		await signIn(member, carol);
		await expect(member).toHaveURL('/sandboxes');

		await rowAction(accounts, 'carol', 'Reset password');
		const reset = page.getByRole('dialog', { name: 'Reset the password of carol' });
		const renewed = { ...carol, password: 'carol, second of her name' };
		await reset.getByLabel('New password').fill(renewed.password);
		await reset.getByRole('button', { name: 'Reset password' }).click();
		await expect(reset).toBeHidden();
		await expect(page.getByText('Reset the password of carol')).toBeVisible();

		await context.clearCookies();
		await member.goto('/sign-in');
		await signIn(member, carol);
		await expect(form.getByRole('alert')).toHaveText('wrong username or password');
		await signIn(member, renewed);
		await expect(member).toHaveURL('/sandboxes');

		await rowAction(accounts, 'carol', 'Delete user');
		const confirm = page.getByRole('alertdialog', { name: 'Delete carol?' });
		await expect(confirm).toContainText('sandboxes, SSH keys and environment');
		const remove = confirm.getByRole('button', { name: 'Delete user' });
		await expect(remove).toBeDisabled();
		await confirm.getByLabel('Type carol to confirm').fill('carol');
		await remove.click();
		await expect(confirm).toBeHidden();
		await expect(accounts.getByRole('row', { name: /carol/ })).toHaveCount(0);

		for (const path of ['/admin/people', '/admin/sandboxes']) {
			await page.goto(path);
			await loaded(page);
			await expect(page.getByRole('row', { name: /carol/ })).toHaveCount(0);
		}

		await member.reload();
		await expect(member.getByRole('alert')).toContainText('this account was deleted');
		await context.close();
	});

	test("sees what runs a member's sandbox from All sandboxes", async ({ page, browser }) => {
		test.setTimeout(120_000);
		const dana = { username: 'dana', password: 'dana, keeper of sandboxes' };
		const made = await page.request.post('/auth/local/accounts', {
			data: { ...dana, role: 'member' }
		});
		expect(made.ok(), await made.text()).toBe(true);
		const context = await browser.newContext({ storageState: { cookies: [], origins: [] } });
		const member = await context.newPage();
		await member.goto('/sign-in');
		await signIn(member, dana);
		await expect(member).toHaveURL('/sandboxes');
		const created = await member.request.post('/api/sandboxes', { data: { name: 'probe' } });
		expect(created.status(), await created.text()).toBe(201);

		await page.goto('/admin/sandboxes');
		await page.getByRole('link', { name: 'probe.dana details' }).click();
		await expect(page).toHaveURL('/admin/sandboxes/dana/probe');
		await expect(page.getByRole('heading', { level: 1 })).toHaveText('probe');
		const facts = page.locator('dl');
		const fact = (label: string) =>
			facts.locator('div').filter({ has: page.getByText(label, { exact: true }) });
		await expect(fact('Container')).toContainText(/corral-dana-probe-[0-9a-f]{6}/);
		await expect(fact('Home volume')).toContainText(/corral-home-dana-probe-[0-9a-f]{6}/);
		await expect(fact('Image')).toContainText(process.env.CORRAL_SANDBOX_IMAGE!);
		const events = page.getByRole('list', { name: 'Recent events' });
		await expect(async () => {
			await page.getByRole('button', { name: 'Refresh' }).click();
			await expect(fact('State')).toContainText('running', { timeout: 1_000 });
			await expect(events).toContainText('Started', { timeout: 1_000 });
		}).toPass({ timeout: 90_000 });

		const deleted = await member.request.delete('/api/me');
		expect(deleted.ok(), await deleted.text()).toBe(true);
		await context.close();
		await page.getByRole('button', { name: 'Refresh' }).click();
		await expect(page.getByText('dana has no sandbox named probe')).toBeVisible();
	});

	test('cannot disable or delete themselves from People', async ({ page }) => {
		await page.goto('/admin/people');
		const self = page.getByRole('table', { name: 'Users' }).getByRole('row', {
			name: new RegExp(account('admin').username)
		});
		await expect(self).toContainText('You');
		await expect(self.getByRole('button')).toHaveCount(0);
		await self.getByRole('link', { name: 'Account' }).click();
		await expect(page).toHaveURL('/account');
	});

	test('cannot delete their own local account, only from Account', async ({ page }) => {
		const { username } = account('admin');
		await page.goto('/admin/people');
		const accounts = page.getByRole('table', { name: 'Local accounts' });
		await expect(accounts.getByRole('row', { name: new RegExp(username) })).toContainText('You');
		await expect(accounts.getByRole('row', { name: /bob/ })).not.toContainText('You');

		await page.getByRole('button', { name: `Actions for ${username}` }).click();
		await expect(page.getByRole('menuitem', { name: 'Change password' })).toBeVisible();
		await expect(page.getByRole('menuitem', { name: 'Delete user', exact: true })).toHaveCount(0);
		await page.getByRole('menuitem', { name: 'Delete from Account' }).click();
		await expect(page).toHaveURL('/account');
	});
});

test.describe('a member', () => {
	test.use({ role: 'member' });

	test('never sees the Admin group and is sent away from admin pages', async ({ page }) => {
		const admin = account('admin').username;
		for (const path of [
			'/admin/people',
			'/admin/settings',
			'/admin/sandboxes',
			`/admin/sandboxes/${admin}/main`
		]) {
			await page.goto(path);
			await expect(page).toHaveURL('/sandboxes');
			await expect(page.getByRole('navigation', { name: 'Main' })).toBeAttached();
			await expect(page.getByRole('navigation', { name: 'Admin' })).toHaveCount(0);
		}
		const details = await page.request.get(`/api/admin/sandboxes/${admin}/main`);
		expect(details.status()).toBe(403);
	});
});

/** Waits for the page's table, or its empty state, to show what the API answered. */
async function loaded(page: Page) {
	await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
	await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
}

async function rowAction(table: Locator, handle: string, action: string) {
	await table.getByRole('button', { name: `Actions for ${handle}` }).click();
	await table.page().getByRole('menuitem', { name: action, exact: true }).click();
}
