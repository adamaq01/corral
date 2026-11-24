import { execFileSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { request, type APIRequestContext, type Page } from '@playwright/test';
import { expect, session, signIn, test } from './fixtures';

/** Signs `page` in as a new member, whose deletion at the end takes their sandboxes with it. */
async function newMember(page: Page, baseURL: string | undefined, username: string) {
	const member = { username, password: randomBytes(18).toString('base64') };
	const admin = await request.newContext({ baseURL, storageState: session('admin') });
	const created = await admin.post('/auth/local/accounts', { data: { ...member, role: 'member' } });
	expect(created.ok(), await created.text()).toBe(true);
	await admin.dispose();
	await page.goto('/sign-in');
	await signIn(page, member);
	await expect(page).toHaveURL('/sandboxes');
}

async function insideDeleteSetting(api: APIRequestContext): Promise<boolean> {
	const reply = await api.get('/api/me/delete-from-inside');
	expect(reply.ok(), await reply.text()).toBe(true);
	return (await reply.json()).enabled;
}

/** The token Corral wrote into the sandbox's home at its start, read as the sandbox user. */
function sandboxToken(owner: string, sandbox: string): string {
	const container = execFileSync(
		'docker',
		[
			'ps',
			'--quiet',
			'--filter',
			`label=corral.adamaq01.fr/owner=${owner}`,
			'--filter',
			`label=corral.adamaq01.fr/sandbox=${sandbox}`
		],
		{ encoding: 'utf8' }
	).trim();
	expect(container, `the running container of ${sandbox}.${owner}`).not.toBe('');
	const file = execFileSync(
		'docker',
		['exec', container, 'cat', '/home/sandbox/.config/corral/sandbox.json'],
		{ encoding: 'utf8' }
	);
	return JSON.parse(file).token;
}

test('the profile turns corral delete on and off', async ({ page, baseURL }) => {
	await newMember(page, baseURL, 'iris');
	await page.goto('/profile');
	const card = page
		.locator('[data-slot=card]')
		.filter({ hasText: 'Deleting from inside a sandbox' });
	const allow = card.getByRole('switch', { name: 'Allow corral delete in my sandboxes' });
	await expect(card).toContainText('corral delete --confirm <name>');
	await expect(allow).not.toBeChecked();
	expect(await insideDeleteSetting(page.request)).toBe(false);

	await allow.click();
	await expect(page.getByText('corral delete now works in your sandboxes')).toBeVisible();
	expect(await insideDeleteSetting(page.request)).toBe(true);
	await page.reload();
	await expect(allow).toBeChecked();

	await allow.click();
	await expect(page.getByText('corral delete is now refused in your sandboxes')).toBeVisible();
	await page.reload();
	await expect(allow).not.toBeChecked();
	expect(await insideDeleteSetting(page.request)).toBe(false);

	const deleted = await page.request.delete('/api/me');
	expect(deleted.ok(), await deleted.text()).toBe(true);
});

test('a deletion corral delete scheduled shows with a Cancel button that keeps the sandbox', async ({
	page,
	baseURL
}) => {
	test.setTimeout(240_000);
	await newMember(page, baseURL, 'jude');
	const allowed = await page.request.put('/api/me/delete-from-inside', {
		data: { enabled: true }
	});
	expect(allowed.ok(), await allowed.text()).toBe(true);
	const created = await page.request.post('/api/sandboxes', { data: { name: 'main' } });
	expect(created.status(), await created.text()).toBe(201);
	await page.goto('/sandboxes/main');
	await expect(page.getByText('Running', { exact: true })).toBeVisible({ timeout: 90_000 });

	const inside = await request.newContext({
		baseURL,
		extraHTTPHeaders: { Authorization: `Bearer ${sandboxToken('jude', 'main')}` }
	});
	const scheduled = await inside.post('/api/sandbox/delete', { data: { confirm: 'main' } });
	expect(scheduled.status(), await scheduled.text()).toBe(202);
	const { deleteAt } = await scheduled.json();
	await inside.dispose();

	await page.reload();
	const alert = page.getByRole('alert').filter({ hasText: 'Deletion scheduled for' });
	await expect(alert).toBeVisible();
	await expect(alert).toContainText('corral delete in this sandbox scheduled its deletion');
	await expect(page.getByText('Sleeping', { exact: true })).toBeVisible({ timeout: 60_000 });
	await expect(page.getByRole('button', { name: 'Wake' })).toHaveCount(0);
	await expect(
		page.getByText('Connections are refused until you cancel the deletion.')
	).toBeVisible();

	await page
		.getByRole('navigation', { name: 'Main' })
		.getByRole('link', { name: 'Sandboxes' })
		.click();
	const row = page.getByRole('list', { name: 'Sandboxes' }).getByRole('listitem');
	await expect(row.getByText('Deletion scheduled')).toBeVisible();
	await row.getByRole('link').click();

	expect((await (await page.request.get('/api/sandboxes/main')).json()).deleteAt).toBe(deleteAt);
	await page.getByRole('button', { name: 'Cancel deletion' }).click();
	await expect(page.getByText('Cancelled the deletion of main')).toBeVisible();
	await expect(alert).toHaveCount(0);
	await expect(page.getByRole('button', { name: 'Wake' })).toBeVisible();
	const kept = await (await page.request.get('/api/sandboxes/main')).json();
	expect([kept.state, kept.deleteAt]).toEqual(['sleeping', null]);

	const deleted = await page.request.delete('/api/me');
	expect(deleted.ok(), await deleted.text()).toBe(true);
});
