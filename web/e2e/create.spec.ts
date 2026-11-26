import { expect, test } from './fixtures';

test.use({ role: 'member' });

test('a sandbox name outside the rule is refused before anything is sent', async ({ page }) => {
	const sent: string[] = [];
	await page.route('**/api/sandboxes', (route) => {
		if (route.request().method() === 'GET') {
			return route.fulfill({ json: [] });
		}
		sent.push(route.request().method());
		return route.abort();
	});
	await page.goto('/sandboxes');
	await page.getByRole('button', { name: 'New sandbox' }).first().click();
	const dialog = page.getByRole('dialog', { name: 'New sandbox' });
	const name = dialog.getByLabel('Name');
	const create = dialog.getByRole('button', { name: 'Create sandbox' });

	await name.fill('Walk 1');
	await expect(dialog.getByRole('alert')).toHaveText(
		'Up to 32 lowercase letters, digits and dashes.'
	);
	await expect(name).toHaveAttribute('aria-invalid', 'true');
	await expect(create).toBeDisabled();
	await name.press('Enter');
	await expect(dialog).toBeVisible();

	await name.fill('walk-1');
	await expect(dialog.getByRole('alert')).toHaveCount(0);
	await expect(dialog).toContainText('ssh -p');
	await expect(create).toBeEnabled();
	expect(sent).toEqual([]);
});
