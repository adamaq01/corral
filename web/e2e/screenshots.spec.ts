import { test } from './fixtures';
import { inEveryView } from './views';

inEveryView((look) => {
	test('sign-in page', async ({ page }) => {
		await page.goto('/sign-in?error=the+identity+provider+refused');
		await page.getByLabel('Username').waitFor();
		await look(page, 'sign-in', { fullPage: false });
	});

	test.describe('signed in', () => {
		test.use({ role: 'admin' });

		test('shell', async ({ page }) => {
			await page.goto('/sandboxes');
			await page.getByRole('heading', { name: 'Sandboxes' }).waitFor();
			await look(page, 'shell', { fullPage: false });
			if (page.viewportSize()!.width < 768) {
				await page.getByRole('button', { name: 'Toggle Sidebar' }).click();
			}
			await page.getByRole('button', { name: 'Account' }).click();
			await page.getByRole('menuitem', { name: 'Sign out' }).waitFor();
			await look(page, 'shell-menu', { fullPage: false });
		});
	});
});
