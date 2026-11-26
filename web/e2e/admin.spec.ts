import type { Locator, Page } from '@playwright/test';
import { account, expect, signIn, test } from './fixtures';

test.describe('an admin', () => {
	test.use({ role: 'admin' });

	test('runs a local account from creation to deletion', async ({ page, browser }) => {
		const carol = { username: 'carol', password: 'carol, first of her name' };

		await page.goto('/admin/people');
		await page.getByRole('button', { name: 'New local account' }).click();
		const create = page.getByRole('dialog', { name: 'New local account' });
		await create.getByLabel('Username').fill(carol.username);
		await create.getByLabel('Password', { exact: true }).fill(carol.password);
		await expect(create.getByRole('radio', { name: /^Member/ })).toBeChecked();
		await create.getByRole('button', { name: 'Create account' }).click();
		await expect(create).toBeHidden();
		await expect(page.getByText('Created carol, who can sign in now')).toBeVisible();
		const people = page.getByRole('table', { name: 'People' });
		const row = people.getByRole('row', { name: /carol/ });
		await expect(row).toContainText('Not signed in yet');
		await expect(row).toContainText('Local account');
		await expect(row).toContainText('Member');

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
		await expect(row).toContainText('Active');
		await expect(row).toContainText('Local account');
		await rowAction(people, 'carol', 'Disable user');
		const disable = page.getByRole('alertdialog', { name: 'Disable carol?' });
		await expect(disable).toContainText('Their sandboxes, SSH keys and environment stay');
		await disable.getByRole('button', { name: 'Disable user' }).click();
		await expect(disable).toBeHidden();
		await expect(row).toContainText('Disabled');

		await member.reload();
		await expect(member).toHaveURL(/^[^?]*\/sign-in\?error=/);
		await expect(member.getByRole('alert')).toContainText('this account is disabled');
		const form = member.getByRole('form', { name: 'Local account' });
		await signIn(member, carol);
		await expect(form.getByRole('alert')).toHaveText('this account is disabled');

		await rowAction(people, 'carol', 'Enable user');
		await expect(row).toContainText('Active');
		await expect(page.getByText('Enabled carol')).toBeVisible();
		await signIn(member, carol);
		await expect(member).toHaveURL('/sandboxes');

		await rowAction(people, 'carol', 'Reset password');
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

		await rowAction(people, 'carol', 'Delete user');
		const confirm = page.getByRole('alertdialog', { name: 'Delete carol?' });
		await expect(confirm).toContainText('sandboxes, SSH keys and environment');
		const remove = confirm.getByRole('button', { name: 'Delete user' });
		await expect(remove).toBeDisabled();
		await confirm.getByLabel('Type carol to confirm').fill('carol');
		await remove.click();
		await expect(confirm).toBeHidden();
		await expect(page.getByText('Deleted carol')).toBeVisible();
		await expect(row).toHaveCount(0);

		for (const path of ['/admin/people', '/admin/sandboxes']) {
			await page.goto(path);
			await loaded(page);
			await expect(page.getByRole('row', { name: /carol/ })).toHaveCount(0);
		}

		await member.reload();
		await expect(member.getByRole('alert')).toContainText('this account was deleted');
		await context.close();
	});

	test('deletes a local account no one has signed in with', async ({ page }) => {
		const made = await page.request.post('/auth/local/accounts', {
			data: { username: 'erin', password: 'erin, never signed in', role: 'member' }
		});
		expect(made.ok(), await made.text()).toBe(true);

		await page.goto('/admin/people');
		const people = page.getByRole('table', { name: 'People' });
		const row = people.getByRole('row', { name: /erin/ });
		await expect(row).toContainText('Not signed in yet');
		await rowAction(people, 'erin', 'Delete account');
		const confirm = page.getByRole('alertdialog', { name: 'Delete erin?' });
		await confirm.getByLabel('Type erin to confirm').fill('erin');
		await confirm.getByRole('button', { name: 'Delete account' }).click();
		await expect(confirm).toBeHidden();
		await expect(page.getByText('Deleted erin')).toBeVisible();
		await expect(row).toHaveCount(0);

		const accounts = await page.request.get('/auth/local/accounts');
		expect(
			(await accounts.json()).map((each: { username: string }) => each.username)
		).not.toContain('erin');
	});

	test('gives a member a sandbox allowance and takes it back', async ({ page }) => {
		await page.goto('/admin/people');
		const people = page.getByRole('table', { name: 'People' });
		const bob = people.getByRole('row', { name: /bob/ });
		await expect(bob).toContainText('up to 5');

		await rowAction(people, 'bob', 'Sandbox allowance…');
		const dialog = page.getByRole('dialog', { name: 'Sandbox allowance for bob' });
		await expect(dialog.getByRole('radio', { name: 'The usual limit' })).toBeChecked();
		await dialog.getByLabel('Sandboxes', { exact: true }).fill('4');
		await dialog.getByRole('button', { name: 'Save allowance' }).click();
		await expect(dialog).toBeHidden();
		await expect(page.getByText('bob may have up to 4 sandboxes')).toBeVisible();
		await expect(bob).toContainText('up to 4');

		await page.reload();
		await expect(bob).toContainText('up to 4');
		await rowAction(people, 'bob', 'Sandbox allowance…');
		await expect(dialog.getByLabel('Sandboxes', { exact: true })).toHaveValue('4');
		await dialog.getByRole('radio', { name: 'The usual limit' }).click();
		await dialog.getByRole('button', { name: 'Save allowance' }).click();
		await expect(dialog).toBeHidden();
		await expect(page.getByText('bob has the usual limit again')).toBeVisible();
		await expect(bob).toContainText('up to 5');
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

		await page.goto('/admin/people');
		await page.getByRole('table', { name: 'People' }).getByRole('link', { name: 'dana' }).click();
		await expect(page).toHaveURL('/admin/sandboxes?owner=dana');
		const owner = page.getByLabel('Owner');
		await expect(owner).toHaveValue('dana');
		await owner.selectOption('Everyone');
		await expect(page).toHaveURL('/admin/sandboxes');
		await owner.selectOption('dana');
		await expect(page).toHaveURL('/admin/sandboxes?owner=dana');
		const sandboxes = page.getByRole('table', { name: 'All sandboxes' });
		const probe = sandboxes.getByRole('row', { name: /probe/ });
		await rowAction(sandboxes, 'probe.dana', 'Details');
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

		await page.goBack();
		await expect(owner).toHaveValue('dana');
		await rowAction(sandboxes, 'probe.dana', 'Sleep');
		await expect(page.getByText('Put probe.dana to sleep')).toBeVisible();
		await expect(probe).toContainText('Sleeping');
		await rowAction(sandboxes, 'probe.dana', 'Wake');
		await expect(page.getByText('Woke probe.dana')).toBeVisible();
		await expect(probe).toContainText('Running');

		await rowAction(sandboxes, 'probe.dana', 'Delete sandbox…');
		const confirm = page.getByRole('alertdialog', { name: 'Delete probe.dana?' });
		await expect(confirm).toContainText("dana's sandbox probe");
		const remove = confirm.getByRole('button', { name: 'Delete sandbox' });
		await expect(remove).toBeDisabled();
		await confirm.getByLabel('Type probe to confirm').fill('probe');
		await remove.click();
		await expect(confirm).toBeHidden();
		await expect(page.getByText('Deleted probe.dana')).toBeVisible();
		await expect(probe).toHaveCount(0);
		await expect(page.getByText('dana has no sandboxes')).toBeVisible();
		await expect(owner).toHaveValue('dana');

		const deleted = await member.request.delete('/api/me');
		expect(deleted.ok(), await deleted.text()).toBe(true);
		await context.close();
	});

	test('cannot disable or delete themselves from People', async ({ page }) => {
		await page.goto('/admin/people');
		const self = page.getByRole('table', { name: 'People' }).getByRole('row', {
			name: new RegExp(account('admin').username)
		});
		await expect(self).toContainText('You');
		await expect(self.getByRole('button')).toHaveCount(0);
		await self.getByRole('link', { name: 'Account' }).click();
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

async function rowAction(table: Locator, row: string, action: string) {
	await table.getByRole('button', { name: `Actions for ${row}` }).click();
	await table.page().getByRole('menuitem', { name: action, exact: true }).click();
}
