import { account, expect, signIn, test } from './fixtures';

// The local provider refuses a network with five logins failed or still in flight, so this
// file's logins run one at a time.
test.describe.configure({ mode: 'default' });

test('signs in with a local account and never shows the sign-in page again', async ({ page }) => {
	await page.goto('/');
	await expect(page).toHaveURL('/sign-in');

	await signIn(page, account('member'));
	await expect(page).toHaveURL('/sandboxes');
	await expect(page.getByRole('heading', { name: 'Sandboxes' })).toBeVisible();
	await expect(page.getByRole('button', { name: 'Account' })).toContainText('bob');

	await page.goto('/sign-in');
	await expect(page).toHaveURL('/sandboxes');
});

test('a wrong password shows the error and keeps the user on the sign-in page', async ({
	page
}) => {
	await page.goto('/sign-in');
	await signIn(page, { username: account('member').username, password: 'not the password' });

	await expect(page.getByRole('alert')).toHaveText('wrong username or password');
	await expect(page).toHaveURL('/sign-in');
	await expect(page.getByLabel('Password')).toBeEmpty();
});

test("a failed redirect login's error is shown", async ({ page }) => {
	await page.goto('/sign-in?error=the+identity+provider+refused');

	await expect(page.getByRole('alert')).toContainText('the identity provider refused');
});

test('a signed-out deep link returns there after signing in', async ({ page }) => {
	await page.goto('/setup?tab=all');
	await expect(page).toHaveURL('/sign-in?next=%2Fsetup%3Ftab%3Dall');

	await signIn(page, account('member'));
	await expect(page).toHaveURL('/setup?tab=all');
	await expect(page.getByRole('heading', { name: 'Setup' })).toBeVisible();
});

test.describe('signed in', () => {
	test.use({ role: 'member' });

	test('reloading a deep link stays on that page', async ({ page }) => {
		await page.goto('/account');
		await expect(page.getByRole('heading', { name: 'Account' })).toBeVisible();

		await page.reload();
		await expect(page).toHaveURL('/account');
		await expect(page.getByRole('heading', { name: 'Account' })).toBeVisible();
		await expect(page).toHaveTitle('Account · Corral');
	});

	test('signing out ends the session', async ({ page }) => {
		await page.goto('/sandboxes');

		await page.getByRole('button', { name: 'Account' }).click();
		await page.getByRole('menuitem', { name: 'Sign out' }).click();
		await expect(page).toHaveURL('/sign-in');

		await page.goto('/account');
		await expect(page).toHaveURL('/sign-in?next=%2Faccount');
		expect((await page.request.get('/api/me')).status()).toBe(401);
	});
});
