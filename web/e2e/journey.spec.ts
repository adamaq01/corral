import { execFileSync, spawn } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { request } from '@playwright/test';
import { expect, session, signIn, test } from './fixtures';

const keyDir = mkdtempSync(join(tmpdir(), 'corral-web-journey-'));
test.afterAll(() => rmSync(keyDir, { recursive: true, force: true }));

/**
 * Runs `remote` over `command`, an SSH command as the UI shows it, with the test's key and
 * ContainerSSH's host key pinned.
 */
function overSsh(command: string, key: string, remote: string): Promise<string> {
	const [program, ...args] = command.split(' ');
	const options = [
		'-F',
		'none',
		'-i',
		key,
		'-o',
		'IdentitiesOnly=yes',
		'-o',
		'IdentityAgent=none',
		'-o',
		'BatchMode=yes',
		'-o',
		`UserKnownHostsFile=${process.env.CORRAL_E2E_KNOWN_HOSTS}`,
		'-o',
		'StrictHostKeyChecking=yes',
		'-o',
		'ConnectTimeout=10',
		'-o',
		'LogLevel=ERROR'
	];
	return new Promise((resolve, reject) => {
		const ssh = spawn(program, [...options, ...args, remote], {
			stdio: ['ignore', 'pipe', 'pipe']
		});
		let [stdout, stderr] = ['', ''];
		ssh.stdout.on('data', (data) => (stdout += data));
		ssh.stderr.on('data', (data) => (stderr += data));
		ssh.once('error', reject);
		ssh.once('exit', (code) =>
			code === 0
				? resolve(stdout.trim())
				: reject(new Error(`${command} exited ${code}: ${stderr}`))
		);
	});
}

function containersOf(handle: string): string[] {
	return execFileSync(
		'docker',
		['ps', '--all', '--quiet', '--filter', `label=corral.adamaq01.fr/owner=${handle}`],
		{ encoding: 'utf8' }
	)
		.split('\n')
		.filter(Boolean);
}

test.use({ permissions: ['clipboard-read', 'clipboard-write'] });

test('a member keys, creates, reaches over SSH, configures and deletes a sandbox, then leaves', async ({
	page,
	baseURL
}) => {
	test.setTimeout(240_000);
	const member = { username: 'dora', password: randomBytes(18).toString('base64') };
	const admin = await request.newContext({ baseURL, storageState: session('admin') });
	const created = await admin.post('/auth/local/accounts', { data: { ...member, role: 'member' } });
	expect(created.ok(), await created.text()).toBe(true);
	await admin.dispose();

	await test.step('sign in', async () => {
		await page.goto('/sign-in');
		await signIn(page, member);
		await expect(page).toHaveURL('/sandboxes');
		await expect(page.getByText('No sandboxes yet')).toBeVisible();
	});

	const key = join(keyDir, 'id_ed25519');
	await test.step('add an SSH key', async () => {
		execFileSync('ssh-keygen', ['-q', '-t', 'ed25519', '-N', '', '-C', 'dora@laptop', '-f', key]);
		const fingerprint = execFileSync('ssh-keygen', ['-lf', `${key}.pub`], {
			encoding: 'utf8'
		}).split(' ')[1];

		await page
			.getByRole('navigation', { name: 'Main' })
			.getByRole('link', { name: 'SSH keys' })
			.click();
		await page.getByRole('button', { name: 'Add key' }).first().click();
		const dialog = page.getByRole('dialog', { name: 'Add an SSH key' });
		await dialog.getByLabel('Public key').fill('not a key');
		await dialog.getByRole('button', { name: 'Add key' }).click();
		await expect(dialog.getByRole('alert')).toContainText('OpenSSH public key');

		await dialog.getByLabel('Public key').fill(readFileSync(`${key}.pub`, 'utf8'));
		await expect(dialog.getByLabel('Label')).toHaveValue('dora@laptop');
		await dialog.getByLabel('Label').fill('laptop');
		await dialog.getByRole('button', { name: 'Add key' }).click();
		await expect(dialog).toBeHidden();
		const row = page.getByRole('listitem').filter({ hasText: 'laptop' });
		await expect(row).toContainText(fingerprint);
	});

	let sshCommand = '';
	await test.step('create a sandbox and see it reach running', async () => {
		await page.getByRole('link', { name: 'Sandboxes' }).first().click();
		await page.getByRole('button', { name: 'New sandbox' }).click();
		const dialog = page.getByRole('dialog', { name: 'New sandbox' });
		await expect(dialog.getByLabel('Name')).toHaveValue('main');
		await dialog.getByRole('button', { name: 'Create sandbox' }).click();
		await expect(page).toHaveURL('/sandboxes/main');
		await expect(page.getByRole('heading', { name: 'main' })).toBeVisible();
		await expect(page.getByText('Running', { exact: true })).toBeVisible({ timeout: 90_000 });

		await page.getByRole('button', { name: 'Copy SSH', exact: true }).click();
		await expect(page.getByText('Copied')).toBeVisible();
		sshCommand = await page.evaluate(() => navigator.clipboard.readText());
		expect(sshCommand).toMatch(/^ssh -p \d+ main\.dora@127\.0\.0\.1$/);
	});

	await test.step('SSH in with the copied command and the added key', async () => {
		expect(await overSsh(sshCommand, key, 'id -un')).toBe('sandbox');
	});

	const token = `journey-${randomBytes(12).toString('hex')}`;
	await test.step('set a credential', async () => {
		await page
			.getByRole('navigation', { name: 'Main' })
			.getByRole('link', { name: 'Credentials' })
			.click();
		await page.getByRole('button', { name: 'Add credential' }).first().click();
		const dialog = page.getByRole('dialog', { name: 'Add a credential' });
		await dialog.getByLabel('Name').fill('journey_token');
		await expect(dialog.getByLabel('Name')).toHaveValue('JOURNEY_TOKEN');
		await dialog.getByLabel('Value', { exact: true }).fill(token);
		await dialog.getByRole('button', { name: 'Save credential' }).click();
		await expect(dialog).toBeHidden();
		await expect(page.getByRole('listitem').filter({ hasText: 'JOURNEY_TOKEN' })).toBeVisible();
		await expect(page.locator('body')).not.toContainText(token);
	});

	await test.step('restart, and see the value in a new SSH session', async () => {
		await page
			.getByRole('navigation', { name: 'Main' })
			.getByRole('link', { name: 'Sandboxes' })
			.click();
		await page.getByRole('link', { name: /main/ }).click();
		await expect(page.getByText('Running', { exact: true })).toBeVisible();
		await page.getByRole('button', { name: 'Restart' }).click();
		const restarting = page.getByRole('button', { name: 'Restarting…' });
		await expect(restarting).toBeVisible();
		await expect(restarting).toBeHidden({ timeout: 60_000 });
		await expect(page.getByText('Running', { exact: true })).toBeVisible({ timeout: 90_000 });
		expect(await overSsh(sshCommand, key, 'printenv JOURNEY_TOKEN')).toBe(token);
	});

	await test.step('delete the sandbox', async () => {
		expect(containersOf('dora')).toHaveLength(1);
		await page.getByRole('button', { name: 'Delete sandbox' }).click();
		const dialog = page.getByRole('alertdialog', { name: 'Delete main?' });
		await expect(dialog).toContainText('remove the machine from Herdr');
		await dialog.getByRole('button', { name: 'Delete sandbox' }).click();
		await expect(page).toHaveURL('/sandboxes');
		await expect(page.getByText('No sandboxes yet')).toBeVisible();
		expect(containersOf('dora')).toEqual([]);
	});

	await test.step('delete the account', async () => {
		await page.getByRole('button', { name: 'Account' }).click();
		await page.getByRole('menuitem', { name: 'Profile' }).click();
		await expect(page).toHaveURL('/profile');
		await page.getByRole('button', { name: 'Delete account' }).click();
		const dialog = page.getByRole('alertdialog', { name: 'Delete your account?' });
		const confirm = dialog.getByRole('button', { name: 'Delete account' });
		await expect(confirm).toBeDisabled();
		await dialog.getByLabel('Type dora to confirm').fill('dora');
		await confirm.click();
		await expect(page).toHaveURL('/sign-in');

		await signIn(page, member);
		await expect(page.getByRole('alert')).toHaveText('wrong username or password');
	});
});
