import type { Locator, Page } from '@playwright/test';
import type { User } from '../src/lib/api/generated/index.js';
import { account, expect, test } from './fixtures';

/** Presses Tab until `target` has focus, as someone without a mouse would. */
async function tabTo(page: Page, target: Locator) {
	for (let stop = 0; stop < 30; stop++) {
		await page.keyboard.press('Tab');
		if (await target.evaluate((element) => element === document.activeElement)) {
			return;
		}
	}
	throw new Error(`Tab never reached ${target}`);
}

test('signs in, moves around and signs out with the keyboard alone', async ({ page }) => {
	const { username, password } = account('member');
	await page.goto('/sign-in');
	await tabTo(page, page.getByLabel('Username'));
	await page.keyboard.type(username);
	await page.keyboard.press('Tab');
	await page.keyboard.type(password);
	await page.keyboard.press('Enter');
	await expect(page).toHaveURL('/sandboxes');

	await tabTo(
		page,
		page.getByRole('navigation', { name: 'Main' }).getByRole('link', { name: 'Account' })
	);
	await page.keyboard.press('Enter');
	await expect(page).toHaveURL('/account');

	await tabTo(page, page.getByRole('button', { name: /^Account/ }));
	await page.keyboard.press('Enter');
	await expect(page.getByRole('menuitem', { name: 'Account' })).toBeFocused();
	await page.keyboard.press('End');
	await expect(page.getByRole('menuitem', { name: 'Sign out' })).toBeFocused();
	await page.keyboard.press('Enter');
	await expect(page).toHaveURL('/sign-in');
});

test.describe('a member', () => {
	test.use({ role: 'member' });

	test('opens a dialog from the keyboard and leaves it with Escape', async ({ page }) => {
		await page.route('**/api/keys', (route) => route.fulfill({ json: [] }));
		await page.goto('/account');
		const add = page.getByRole('button', { name: 'Add key' });
		await tabTo(page, add);
		await page.keyboard.press('Enter');
		const dialog = page.getByRole('dialog', { name: 'Add an SSH key' });
		await expect(dialog.getByLabel('Public key')).toBeFocused();
		await page.keyboard.press('Escape');
		await expect(dialog).toBeHidden();
		await expect(add).toBeFocused();
	});
});

test.describe('an admin', () => {
	test.use({ role: 'admin' });

	test('reaches a row action and its confirmation from the keyboard', async ({ page }) => {
		const bob: User = {
			id: '0b9c6f0e-1f3e-4a4e-9a52-6a1c0f1d2e3f',
			handle: 'bob',
			email: null,
			picture: null,
			role: 'member',
			status: 'active',
			createdAt: '2026-11-02T09:30:00Z',
			provider: 'oidc',
			subject: 'bob',
			allowance: null
		};
		await page.route('**/api/admin/users', (route) => route.fulfill({ json: [bob] }));
		await page.route('**/auth/local/accounts', (route) => route.fulfill({ json: [] }));
		await page.goto('/admin/people');
		const actions = page
			.getByRole('table', { name: 'People' })
			.getByRole('button', { name: 'Actions for bob' });
		await tabTo(page, actions);
		await page.keyboard.press('Enter');
		await expect(page.getByRole('menuitem', { name: 'Their sandboxes' })).toBeFocused();
		await page.keyboard.press('End');
		await expect(page.getByRole('menuitem', { name: 'Delete user' })).toBeFocused();
		await page.keyboard.press('Enter');
		const confirm = page.getByRole('alertdialog', { name: 'Delete bob?' });
		await expect
			.poll(() => confirm.evaluate((dialog) => dialog.contains(document.activeElement)))
			.toBe(true);
		await page.keyboard.press('Escape');
		await expect(confirm).toBeHidden();
		await expect(actions).toBeFocused();
	});
});
