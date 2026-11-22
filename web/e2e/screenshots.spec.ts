import { join } from 'node:path';
import type { Page } from '@playwright/test';
import { test } from './fixtures';

const directory = process.env.CORRAL_SCREENSHOTS;

test.skip(!directory, 'CORRAL_SCREENSHOTS names the directory to take screenshots into');

const widths = { desktop: 1280, phone: 390 };

for (const scheme of ['light', 'dark'] as const) {
	for (const [device, width] of Object.entries(widths)) {
		test.describe(`${scheme}, ${device}`, () => {
			test.use({
				colorScheme: scheme,
				viewport: { width, height: device === 'phone' ? 844 : 800 }
			});

			const shot = (page: Page, name: string) =>
				page.screenshot({
					path: join(directory!, `${name}-${scheme}-${width}.png`),
					animations: 'disabled'
				});

			test('sign-in page', async ({ page }) => {
				await page.goto('/sign-in?error=the+identity+provider+refused');
				await page.getByLabel('Username').waitFor();
				await shot(page, 'sign-in');
			});

			test.describe('signed in', () => {
				test.use({ role: 'admin' });

				test('shell', async ({ page }) => {
					await page.goto('/sandboxes');
					await page.getByRole('heading', { name: 'Sandboxes' }).waitFor();
					await shot(page, 'shell');
					if (device === 'phone') {
						await page.getByRole('button', { name: 'Toggle Sidebar' }).click();
					}
					await page.getByRole('button', { name: 'Account' }).click();
					await page.getByRole('menuitem', { name: 'Sign out' }).waitFor();
					await shot(page, 'shell-menu');
				});
			});
		});
	}
}
