import { randomBytes } from 'node:crypto';
import { request, type Page, type Route } from '@playwright/test';
import { expect, session, signIn, test } from './fixtures';

/** Holds the requests to `path` until `release` is called, then lets them reach Corral. */
async function hold(page: Page, path: string) {
	let release = () => {};
	const released = new Promise<void>((done) => (release = done));
	await page.route(`**${path}`, async (route: Route) => {
		await released;
		await route.fulfill({ response: await route.fetch() });
	});
	return release;
}

/** Holds the answers to requests to `path`, which Corral handles at once, until `release`. */
async function holdAnswers(page: Page, path: string) {
	let release = () => {};
	const released = new Promise<void>((done) => (release = done));
	await page.route(`**${path}`, async (route: Route) => {
		const response = await route.fetch();
		await released;
		await route.fulfill({ response });
	});
	return release;
}

test('the sandbox page and the list keep up with a power action while it runs and after', async ({
	page,
	baseURL
}) => {
	test.setTimeout(240_000);
	const member = { username: 'erin', password: randomBytes(18).toString('base64') };
	const admin = await request.newContext({ baseURL, storageState: session('admin') });
	const created = await admin.post('/auth/local/accounts', { data: { ...member, role: 'member' } });
	expect(created.ok(), await created.text()).toBe(true);
	await admin.dispose();

	await page.goto('/sign-in');
	await signIn(page, member);
	await expect(page).toHaveURL('/sandboxes');
	const sandbox = await page.request.post('/api/sandboxes', { data: { name: 'main' } });
	expect(sandbox.status(), await sandbox.text()).toBe(201);

	const badge = (scope: Page | ReturnType<Page['getByRole']>, state: string) =>
		scope.getByText(state, { exact: true });

	await test.step('the sandbox page shows a sleep done before its answer arrives', async () => {
		await page.goto('/sandboxes/main');
		await expect(badge(page, 'Running')).toBeVisible({ timeout: 90_000 });
		const release = await holdAnswers(page, '/api/sandboxes/main/stop');
		await page.getByRole('button', { name: 'Sleep' }).click();
		await expect(page.getByRole('button', { name: 'Going to sleep…' })).toBeVisible();
		await expect(page.getByRole('button', { name: 'Delete sandbox' })).toBeDisabled();
		await expect(badge(page, 'Sleeping')).toBeVisible({ timeout: 60_000 });
		const wake = page.getByRole('button', { name: 'Wake' });
		await expect(wake).toBeDisabled();
		release();
		await expect(wake).toBeEnabled();
	});

	await test.step('the list, opened while a wake is in flight, follows it to running', async () => {
		const release = await hold(page, '/api/sandboxes/main/wake');
		await page.getByRole('button', { name: 'Wake' }).click();
		await expect(page.getByRole('button', { name: 'Waking…' })).toBeVisible();
		await expect(page.getByText('Starting main. This usually takes under a minute.')).toBeVisible();
		await expect(page.getByRole('button', { name: 'Make default' })).toHaveCount(0);
		await page
			.getByRole('navigation', { name: 'Main' })
			.getByRole('link', { name: 'Sandboxes' })
			.click();
		const row = page.getByRole('list', { name: 'Sandboxes' }).getByRole('listitem');
		await expect(badge(row, 'Sleeping')).toBeVisible();
		release();
		await expect(badge(row, 'Running')).toBeVisible({ timeout: 90_000 });
	});

	await test.step('a setup change marks the running sandbox until a restart', async () => {
		const created = await page.request.post('/api/setup', {
			data: {
				kind: 'variable',
				target: 'LIFECYCLE_TOKEN',
				secret: true,
				template: false,
				applyToNew: false,
				value: { text: 'one' }
			}
		});
		expect(created.ok(), await created.text()).toBe(true);
		const { id } = await created.json();
		const attached = await page.request.put(`/api/sandboxes/main/setup/${id}`, {
			headers: { 'Content-Type': 'application/json' }
		});
		expect(attached.ok(), await attached.text()).toBe(true);
		await page.goto('/sandboxes/main');
		await expect(page.getByText('Restart to apply your setup changes')).toBeVisible();

		await page
			.getByRole('navigation', { name: 'Main' })
			.getByRole('link', { name: 'Sandboxes' })
			.click();
		const row = page.getByRole('list', { name: 'Sandboxes' }).getByRole('listitem');
		await expect(row).toContainText('Restart to apply your setup changes.');
		await row.getByRole('button', { name: 'Restart main' }).click();
		await expect(row).not.toContainText('Restart to apply', { timeout: 90_000 });
		await expect(badge(row, 'Running')).toBeVisible({ timeout: 90_000 });

		await page.goto('/sandboxes/main');
		await expect(badge(page, 'Running')).toBeVisible();
		await expect(page.getByText('Restart to apply your setup changes')).toBeHidden();
	});

	const deleted = await page.request.delete('/api/me');
	expect(deleted.ok(), await deleted.text()).toBe(true);
});
