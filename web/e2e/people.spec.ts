import type { LocalAccount, User } from '../src/lib/api/generated/index.js';
import { expect, test } from './fixtures';

test.describe('an admin', () => {
	test.use({ role: 'admin' });

	test('sees a local account on the user who signs in with it, whatever their handle', async ({
		page
	}) => {
		const user: User = {
			id: '0b9c6f0e-1f3e-4a4e-9a52-6a1c0f1d2e3f',
			handle: 'adam-2',
			email: null,
			picture: null,
			role: 'member',
			status: 'active',
			createdAt: '2026-11-02T09:30:00Z',
			provider: 'local',
			subject: 'adam',
			allowance: null
		};
		const account: LocalAccount = {
			username: 'adam',
			role: 'member',
			createdAt: '2026-11-01T09:30:00Z',
			yours: false
		};
		await page.route('**/api/admin/users', (route) => route.fulfill({ json: [user] }));
		await page.route('**/auth/local/accounts', (route) => route.fulfill({ json: [account] }));
		await page.goto('/admin/people');

		const people = page.getByRole('table', { name: 'People' });
		await expect(people.getByRole('row', { name: /adam/ })).toHaveCount(1);
		await expect(people).not.toContainText('Not signed in yet');
		await people.getByRole('button', { name: 'Actions for adam-2' }).click();
		const menu = page.getByRole('menu');
		await expect(menu.getByRole('menuitem', { name: 'Reset password' })).toBeVisible();
		await expect(menu.getByRole('menuitem', { name: 'Delete user' })).toBeVisible();
		await expect(menu.getByRole('menuitem', { name: 'Delete account' })).toHaveCount(0);
	});
});
