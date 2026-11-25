import { randomBytes } from 'node:crypto';
import { request } from '@playwright/test';
import { expect, session, signIn, test } from './fixtures';

test('an owner picks how long a sandbox stays awake with no work, and keeps it awake', async ({
	page,
	baseURL
}) => {
	test.setTimeout(180_000);
	const member = { username: 'sleepy', password: randomBytes(18).toString('base64') };
	const admin = await request.newContext({ baseURL, storageState: session('admin') });
	const created = await admin.post('/auth/local/accounts', { data: { ...member, role: 'member' } });
	expect(created.ok(), await created.text()).toBe(true);
	await admin.dispose();

	await page.goto('/sign-in');
	await signIn(page, member);
	await expect(page).toHaveURL('/sandboxes');
	const sandbox = await page.request.post('/api/sandboxes', { data: { name: 'main' } });
	expect(sandbox.status(), await sandbox.text()).toBe(201);
	await page.goto('/sandboxes/main');

	const card = page.locator('[data-slot="card"]', {
		has: page.locator('[data-slot="card-title"]', { hasText: /^Sleep$/ })
	});
	await expect(card.getByText('main sleeps after 30 min with no work')).toBeVisible();

	await test.step('a timeout of its own, then the default again', async () => {
		const timeout = page.getByLabel('Sleep after no work for');
		await timeout.selectOption({ label: '2 h' });
		await expect(page.getByText('main sleeps after 2 h with no work').first()).toBeVisible();
		const saved = await page.request.get('/api/sandboxes/main');
		expect((await saved.json()).idle).toMatchObject({ timeoutSecs: 7200, chosen: true });
		await timeout.selectOption({ label: '30 min (default)' });
		await expect(card.getByText('main sleeps after 30 min with no work')).toBeVisible();
		const reset = await page.request.get('/api/sandboxes/main');
		expect((await reset.json()).idle).toMatchObject({ timeoutSecs: 1800, chosen: false });
	});

	await test.step('kept awake for 4 hours, then no longer', async () => {
		const before = Date.now();
		await card.getByRole('button', { name: '4 hours' }).click();
		await expect(card.getByRole('status')).toContainText('Awake until');
		const kept = (await (await page.request.get('/api/sandboxes/main')).json()).idle;
		const until = new Date(kept.keepAwakeUntil).getTime();
		expect(until).toBeGreaterThanOrEqual(before + 4 * 3600_000 - 1000);
		expect(until).toBeLessThanOrEqual(Date.now() + 4 * 3600_000 + 1000);
		await card.getByRole('button', { name: 'Stop keeping awake' }).click();
		await expect(card.getByRole('button', { name: 'Stop keeping awake' })).toBeHidden();
		const released = (await (await page.request.get('/api/sandboxes/main')).json()).idle;
		expect(released.keepAwakeUntil).toBeNull();
	});
});
