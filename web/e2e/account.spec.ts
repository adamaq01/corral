import { request } from '@playwright/test';
import { expect, session, signIn, test } from './fixtures';

test('a local member changes their password from Account', async ({ page, baseURL }) => {
	const gwen = { username: 'gwen', password: 'gwen, before the change' };
	const changed = { ...gwen, password: 'gwen, after the change' };
	const admin = await request.newContext({ baseURL, storageState: session('admin') });
	const created = await admin.post('/auth/local/accounts', { data: { ...gwen, role: 'member' } });
	expect(created.ok(), await created.text()).toBe(true);
	await admin.dispose();

	await page.goto('/sign-in');
	await signIn(page, gwen);
	await expect(page).toHaveURL('/sandboxes');
	await page.goto('/account');
	await page.getByRole('button', { name: 'Change password' }).click();
	const dialog = page.getByRole('dialog', { name: 'Change your password' });
	const current = dialog.getByLabel('Current password');

	// Every test signs in from the same address, which a real wrong password would count against.
	await page.route(
		'**/auth/local/password',
		(route) => route.fulfill({ status: 403, json: { error: 'wrong password' } }),
		{ times: 1 }
	);
	await current.fill('gwen, a wrong guess');
	await dialog.getByLabel('New password').fill(changed.password);
	await dialog.getByRole('button', { name: 'Change password' }).click();
	await expect(dialog.getByRole('alert')).toHaveText('Wrong password.');
	await expect(current).toHaveAttribute('aria-invalid', 'true');

	await current.fill(gwen.password);
	await expect(dialog.getByRole('alert')).toHaveCount(0);
	await dialog.getByRole('button', { name: 'Change password' }).click();
	await expect(dialog).toBeHidden();
	await expect(page.getByText('Password changed.')).toBeVisible();

	await page.context().clearCookies();
	await page.goto('/sign-in');
	await signIn(page, changed);
	await expect(page).toHaveURL('/sandboxes');

	const deleted = await page.request.delete('/api/me');
	expect(deleted.ok(), await deleted.text()).toBe(true);
});
