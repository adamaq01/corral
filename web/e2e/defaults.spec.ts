import { randomBytes } from 'node:crypto';
import { request } from '@playwright/test';
import { expect, session, signIn, test } from './fixtures';

test('the defaults a member saves on Account preselect the new sandbox dialog', async ({
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

	await page.goto('/account');
	const card = page.locator('[data-slot=card]').filter({ hasText: 'New sandboxes' });
	await expect(card).toContainText('as <name>.fern with a name');
	await expect(card.getByLabel('Image')).toHaveValue('dev');
	await expect(card.getByLabel('Size')).toHaveValue('small');
	const save = card.getByRole('button', { name: 'Save defaults' });
	const reset = card.getByRole('button', { name: 'Reset' });
	await expect(save).toBeDisabled();
	await expect(reset).toHaveCount(0);

	await card.getByLabel('Image').selectOption({ label: 'Local build, again' });
	await expect(card.getByRole('status')).toHaveText('Unsaved changes');
	await reset.click();
	await expect(card.getByLabel('Image')).toHaveValue('dev');
	await expect(card.getByRole('status')).toHaveText('');
	await expect(save).toBeDisabled();

	await card.getByLabel('Image').selectOption({ label: 'Local build, again' });
	const sandboxesLink = page
		.getByRole('navigation', { name: 'Main' })
		.getByRole('link', { name: 'Sandboxes' });
	await sandboxesLink.click();
	const leave = page.getByRole('alertdialog', { name: 'Leave without saving?' });
	await leave.getByRole('button', { name: 'Stay' }).click();
	await expect(page).toHaveURL('/account');
	await expect(card.getByLabel('Image')).toHaveValue('dev-again');

	await card.getByLabel('Size').selectOption({ label: 'Roomy' });
	await save.click();
	await expect(
		page.getByText('Saved your defaults. New sandboxes start from these.')
	).toBeVisible();
	await expect(save).toBeDisabled();
	await expect(card.getByRole('status')).toHaveText('');

	await page.reload();
	await expect(card.getByLabel('Image')).toHaveValue('dev-again');
	await expect(card.getByLabel('Size')).toHaveValue('roomy');

	await card.getByLabel('Size').selectOption({ label: 'Small' });
	await sandboxesLink.click();
	await leave.getByRole('button', { name: 'Leave' }).click();
	await expect(page).toHaveURL('/sandboxes');
	await page.getByRole('button', { name: 'New sandbox' }).click();
	const dialog = page.getByRole('dialog', { name: 'New sandbox' });
	await expect(dialog.getByLabel('Image')).toHaveValue('dev-again');
	await expect(dialog.getByLabel('Size')).toHaveValue('roomy');
	await dialog.getByRole('button', { name: 'Cancel' }).click();

	const deleted = await page.request.delete('/api/me');
	expect(deleted.ok(), await deleted.text()).toBe(true);
});
