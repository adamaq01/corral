import { execFileSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { request } from '@playwright/test';
import { expect, session, signIn, test } from './fixtures';
import { overSsh } from './ssh';

const keyDir = mkdtempSync(join(tmpdir(), 'corral-web-connect-'));
test.afterAll(() => rmSync(keyDir, { recursive: true, force: true }));

test('connecting to a sandbox not made yet makes it from the account defaults, then opens it', async ({
	page,
	baseURL
}) => {
	test.setTimeout(180_000);
	const member = { username: 'gail', password: randomBytes(18).toString('base64') };
	const admin = await request.newContext({ baseURL, storageState: session('admin') });
	const created = await admin.post('/auth/local/accounts', { data: { ...member, role: 'member' } });
	expect(created.ok(), await created.text()).toBe(true);
	await admin.dispose();

	await page.goto('/sign-in');
	await signIn(page, member);
	await expect(page).toHaveURL('/sandboxes');
	const key = join(keyDir, 'id_ed25519');
	execFileSync('ssh-keygen', ['-q', '-t', 'ed25519', '-N', '', '-C', 'gail@laptop', '-f', key]);
	const added = await page.request.post('/api/keys', {
		data: { label: 'laptop', publicKey: readFileSync(`${key}.pub`, 'utf8') }
	});
	expect(added.ok(), await added.text()).toBe(true);
	const defaults = await page.request.put('/api/me/sandbox-defaults', {
		data: { image: 'dev-again', size: 'roomy' }
	});
	expect(defaults.ok(), await defaults.text()).toBe(true);

	const [, port] = /^\[127\.0\.0\.1\]:(\d+) /.exec(
		readFileSync(process.env.CORRAL_E2E_KNOWN_HOSTS!, 'utf8')
	)!;
	const uri = (username: string) => [`ssh://${username}@127.0.0.1:${port}`];

	await test.step('an ssh:// URI to scratch.gail makes scratch and opens it', async () => {
		const login = await overSsh(uri('scratch.gail'), key, 'echo here >~/marker; id -un');
		expect(login, login.stderr).toMatchObject({ code: 0, stdout: 'sandbox' });
		await page.goto('/sandboxes/scratch');
		await expect(page.getByRole('heading', { name: 'scratch' })).toBeVisible();
		await expect(page.getByText('Local build, again · Roomy')).toBeVisible();
		await expect(page.getByText('Running', { exact: true })).toBeVisible();
		await expect(
			page.getByText('Connecting on a terminal to a name you have no sandbox by opens an installer')
		).toBeVisible();
	});

	await test.step('the bare handle reaches the default sandbox, scratch', async () => {
		const login = await overSsh(['-p', port, 'gail@127.0.0.1'], key, 'cat ~/marker');
		expect(login, login.stderr).toMatchObject({ code: 0, stdout: 'here' });
	});

	await test.step('a + in the user name is refused, written either way', async () => {
		for (const destination of [uri('gail+scratch'), ['-p', port, 'gail+scratch@127.0.0.1']]) {
			const login = await overSsh(destination, key, 'true');
			expect(login.code, destination.join(' ')).toBe(255);
			expect(login.stderr).toContain('Permission denied (');
		}
		const listed = await page.request.get('/api/sandboxes');
		expect((await listed.json()).map((sandbox: { name: string }) => sandbox.name)).toEqual([
			'scratch'
		]);
	});

	const deleted = await page.request.delete('/api/me');
	expect(deleted.ok(), await deleted.text()).toBe(true);
});
