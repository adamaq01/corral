import { expect, test } from './fixtures';

test.describe('an admin', () => {
	test.use({ role: 'admin' });

	test('sees the Admin group', async ({ page }) => {
		await page.goto('/sandboxes');

		const admin = page.getByRole('navigation', { name: 'Admin' });
		await expect(admin.getByRole('link')).toHaveText(['Settings', 'People', 'All sandboxes']);
		await admin.getByRole('link', { name: 'People' }).click();
		await expect(page).toHaveURL('/admin/people');
		await expect(page.getByRole('heading', { name: 'People' })).toBeVisible();

		await admin.getByRole('link', { name: 'Settings' }).click();
		await expect(page).toHaveURL('/admin/settings/catalog');
		await expect(page.getByRole('heading', { name: 'Settings' })).toBeVisible();
		const tabs = page.getByRole('navigation', { name: 'Settings' });
		await expect(tabs.getByRole('link')).toHaveText(['Catalog', 'Limits', 'Registries']);
		await expect(tabs.getByRole('link', { name: 'Catalog' })).toHaveAttribute(
			'aria-current',
			'page'
		);
		await tabs.getByRole('link', { name: 'Limits' }).click();
		await expect(page).toHaveURL('/admin/settings/limits');
		await expect(tabs.getByRole('link', { name: 'Limits' })).toHaveAttribute(
			'aria-current',
			'page'
		);
		await expect(admin.getByRole('link', { name: 'Settings' })).toHaveAttribute(
			'aria-current',
			'page'
		);
	});
});

test.describe('a member', () => {
	test.use({ role: 'member' });

	test('sees no Admin group and cannot open admin pages', async ({ page }) => {
		await page.goto('/sandboxes');

		await expect(page.getByRole('navigation', { name: 'Main' }).getByRole('link')).toHaveText([
			'Sandboxes',
			'Environment',
			'Account'
		]);
		await expect(page.getByRole('navigation', { name: 'Admin' })).toHaveCount(0);

		await page.goto('/admin/people');
		await expect(page).toHaveURL('/sandboxes');
	});

	test('gets the system theme until choosing one, and keeps the choice', async ({ page }) => {
		await page.emulateMedia({ colorScheme: 'dark' });
		await page.goto('/sandboxes');
		const html = page.locator('html');
		await expect(html).toHaveClass(/\bdark\b/);

		await page.getByRole('button', { name: 'Theme' }).click();
		await page.getByRole('menuitemradio', { name: 'Light' }).click();
		await expect(html).not.toHaveClass(/\bdark\b/);

		await page.reload();
		await expect(page.getByRole('heading', { name: 'Sandboxes' })).toBeVisible();
		await expect(html).not.toHaveClass(/\bdark\b/);

		await page.getByRole('button', { name: 'Theme' }).click();
		await page.getByRole('menuitemradio', { name: 'System' }).click();
		await expect(html).toHaveClass(/\bdark\b/);
	});

	test.describe('at phone width', () => {
		test.use({ viewport: { width: 390, height: 844 } });

		test('opens the navigation from the header, and a choice closes it', async ({ page }) => {
			await page.goto('/sandboxes');
			const nav = page.getByRole('navigation', { name: 'Main' });
			await expect(nav).toBeHidden();

			await page.getByRole('button', { name: 'Toggle Sidebar' }).click();
			await nav.getByRole('link', { name: 'Account' }).click();
			await expect(page).toHaveURL('/account');
			await expect(nav).toBeHidden();
			await expect(page.getByRole('heading', { name: 'Account' })).toBeVisible();
			const header = page.locator('header', {
				has: page.getByRole('button', { name: 'Toggle Sidebar' })
			});
			await expect(header.getByText('Account')).toBeVisible();
		});
	});
});
