import { expect, test } from './fixtures';

test.use({ role: 'member' });

test('the value being typed shows and hides, and is sent as typed', async ({ page }) => {
	const saved: unknown[] = [];
	await page.route('**/api/credentials', (route) => route.fulfill({ json: [] }));
	await page.route('**/api/credentials/OPENAI_API_KEY', async (route) => {
		saved.push(route.request().postDataJSON());
		await route.fulfill({ status: 204 });
	});
	await page.goto('/credentials');
	await page.getByRole('button', { name: 'Add credential' }).first().click();
	const dialog = page.getByRole('dialog', { name: 'Add a credential' });
	const value = dialog.getByLabel('Value', { exact: true });
	const show = dialog.getByRole('button', { name: 'Show the value' });

	await dialog.getByLabel('Name').fill('OPENAI_API_KEY');
	await value.fill('sk-typed');
	await expect(value).toHaveAttribute('type', 'password');
	await expect(show).toHaveAttribute('aria-pressed', 'false');
	await show.click();
	await expect(value).toHaveAttribute('type', 'text');
	await expect(show).toHaveAttribute('aria-pressed', 'true');
	await value.press('End');
	await value.pressSequentially('-more');
	await expect(value).toHaveValue('sk-typed-more');
	await show.click();
	await expect(value).toHaveAttribute('type', 'password');
	await expect(dialog).toBeVisible();

	await show.click();
	await dialog.getByRole('button', { name: 'Save credential' }).click();
	await expect(dialog).toBeHidden();
	expect(saved).toEqual([{ value: 'sk-typed-more' }]);

	await page.getByRole('button', { name: 'Add credential' }).first().click();
	await expect(value).toHaveValue('');
	await expect(value).toHaveAttribute('type', 'password');
});
