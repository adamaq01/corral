import { randomBytes } from 'node:crypto';
import { request } from '@playwright/test';
import { expect, session, signIn, test } from './fixtures';

test('the defaults a member saves in their profile preselect the new sandbox dialog', async ({
	page,
	baseURL
}) => {
	const member = { username: 'fern', password: randomBytes(18).toString('base64') };
	const admin = await request.newContext({ baseURL, storageState: session('admin') });
	const created = await admin.post('/auth/local/accounts', { data: { ...member, role: 'member' } });
	expect(created.ok(), await created.text()).toBe(true);
	await admin.dispose();

	await page.goto('/sign-in');
	await signIn(page, member);
	await expect(page).toHaveURL('/sandboxes');

	await page.goto('/profile');
	const card = page.locator('[data-slot=card]').filter({ hasText: 'New sandboxes' });
	await expect(card).toContainText('as <name>.fern with a name');
	await expect(card.getByLabel('Image')).toHaveValue('dev');
	await expect(card.getByLabel('Size')).toHaveValue('small');
	await card.getByLabel('Image').selectOption({ label: 'Local build, again' });
	await card.getByLabel('Size').selectOption({ label: 'Roomy' });
	await card.getByRole('button', { name: 'Save defaults' }).click();
	await expect(card.getByRole('status')).toHaveText('Saved. New sandboxes start from these.');

	await page.reload();
	await expect(card.getByLabel('Image')).toHaveValue('dev-again');
	await expect(card.getByLabel('Size')).toHaveValue('roomy');

	await page
		.getByRole('navigation', { name: 'Main' })
		.getByRole('link', { name: 'Sandboxes' })
		.click();
	await page.getByRole('button', { name: 'New sandbox' }).click();
	const dialog = page.getByRole('dialog', { name: 'New sandbox' });
	await expect(dialog.getByLabel('Image')).toHaveValue('dev-again');
	await expect(dialog.getByLabel('Size')).toHaveValue('roomy');
	await dialog.getByRole('button', { name: 'Cancel' }).click();

	const deleted = await page.request.delete('/api/me');
	expect(deleted.ok(), await deleted.text()).toBe(true);
});
