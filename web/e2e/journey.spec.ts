import { execFileSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { request } from '@playwright/test';
import { expect, session, signIn, test } from './fixtures';
import { sshOutput } from './ssh';

const keyDir = mkdtempSync(join(tmpdir(), 'corral-web-journey-'));
test.afterAll(() => rmSync(keyDir, { recursive: true, force: true }));

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
			.getByRole('link', { name: 'Account' })
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
		const row = page
			.getByRole('list', { name: 'SSH keys' })
			.getByRole('listitem')
			.filter({ hasText: 'laptop' });
		await expect(row).toContainText(fingerprint);
	});

	const token = `journey-${randomBytes(12).toString('hex')}`;
	await test.step('add a secret variable and a file', async () => {
		await page
			.getByRole('navigation', { name: 'Main' })
			.getByRole('link', { name: 'Setup' })
			.click();
		await page.getByRole('button', { name: 'Add variable' }).first().click();
		const dialog = page.getByRole('dialog', { name: 'Add a variable' });
		await dialog.getByLabel('Name').fill('JOURNEY_TOKEN');
		await dialog.getByRole('switch', { name: 'Secret' }).click();
		await dialog.getByLabel('Value', { exact: true }).fill(token);
		await dialog.getByRole('button', { name: 'Save variable' }).click();
		await expect(dialog).toBeHidden();
		await expect(page.getByRole('heading', { name: 'JOURNEY_TOKEN' })).toBeVisible();
		await expect(page.getByRole('img', { name: /^Secret value/ })).toContainText(
			`${token.slice(0, 4)}••••••••••••${token.slice(-2)}`
		);
		await expect(page.locator('body')).not.toContainText(token);

		await page.getByRole('link', { name: 'Setup' }).first().click();
		await page.getByRole('button', { name: 'Add file' }).first().click();
		const file = page.getByRole('dialog', { name: 'Add a file' });
		await file.getByLabel('Path in the home').fill('.journey/notes');
		await file.getByLabel('Value', { exact: true }).fill('token={{ JOURNEY_TOKEN }}');
		await file.getByRole('switch', { name: 'Template' }).click();
		await file.getByRole('switch', { name: 'Apply to new sandboxes' }).click();
		await file.getByRole('button', { name: 'Save file' }).click();
		await expect(file).toBeHidden();
		await expect(page.getByRole('heading', { name: '~/.journey/notes' })).toBeVisible();
	});

	await test.step('save a git identity', async () => {
		await page
			.getByRole('navigation', { name: 'Main' })
			.getByRole('link', { name: 'Setup' })
			.click();
		await page.getByRole('button', { name: 'Set up Git identity' }).click();
		const dialog = page.getByRole('dialog', { name: 'Git identity' });
		await dialog.getByLabel('Name', { exact: true }).fill('Dora Journey');
		await dialog.getByLabel('Email', { exact: true }).fill('dora@example.com');
		await dialog.getByRole('button', { name: 'Save identity' }).click();
		await expect(dialog).toBeHidden();
		await expect(page.getByRole('list', { name: 'Presets' })).toContainText(
			'Dora Journey <dora@example.com>'
		);
	});

	await test.step('save an MCP server', async () => {
		await page.getByRole('button', { name: 'Set up MCP servers' }).click();
		const dialog = page.getByRole('dialog', { name: 'MCP servers' });
		await dialog.getByLabel('Name', { exact: true }).fill('fetch');
		await dialog.getByLabel('Command', { exact: true }).fill('uvx');
		await dialog.getByLabel('Arguments', { exact: true }).fill('mcp-server-fetch');
		await dialog.getByRole('button', { name: 'Save servers' }).click();
		await expect(dialog).toBeHidden();
		await expect(page.getByRole('list', { name: 'Presets' })).toContainText('1 server: fetch');
	});

	let destination: string[] = [];
	await test.step('create a sandbox and see it reach running', async () => {
		await page.getByRole('link', { name: 'Sandboxes' }).first().click();
		await page.getByRole('button', { name: 'New sandbox' }).click();
		const dialog = page.getByRole('dialog', { name: 'New sandbox' });
		await expect(dialog.getByLabel('Name')).toHaveValue('main');
		const variable = dialog.getByRole('checkbox', { name: 'JOURNEY_TOKEN' });
		await expect(variable).toBeChecked();
		await expect(dialog.getByRole('checkbox', { name: '~/.journey/notes' })).not.toBeChecked();
		await expect(dialog.getByRole('checkbox', { name: '~/.gitconfig' })).toBeChecked();
		await expect(
			dialog.getByRole('checkbox', { name: '~/.config/corral-agents/mcp.json' })
		).toBeChecked();
		await variable.click();
		await dialog.getByRole('button', { name: 'Create sandbox' }).click();
		await expect(page).toHaveURL('/sandboxes/main');
		await expect(page.getByRole('heading', { name: 'main' })).toBeVisible();
		const attached = page.getByRole('list', { name: 'Attached to main' });
		await expect(attached.getByRole('listitem')).toHaveCount(2);
		await expect(attached).toContainText('~/.gitconfig');
		await expect(attached).toContainText('~/.config/corral-agents/mcp.json');
		await expect(page.getByText('Running', { exact: true })).toBeVisible({ timeout: 90_000 });

		await page.getByRole('button', { name: 'Copy SSH', exact: true }).click();
		await expect(page.getByText('Copied')).toBeVisible();
		const sshCommand = await page.evaluate(() => navigator.clipboard.readText());
		expect(sshCommand).toMatch(/^ssh -p \d+ main\.dora@127\.0\.0\.1$/);
		destination = sshCommand.split(' ').slice(1);
	});

	await test.step('SSH in with the copied command and the added key', async () => {
		expect(await sshOutput(destination, key, 'id -un')).toBe('sandbox');
		expect(await sshOutput(destination, key, 'printenv JOURNEY_TOKEN || echo absent')).toBe(
			'absent'
		);
		expect(await sshOutput(destination, key, 'git config user.name')).toBe('Dora Journey');
		const fetch = { command: 'uvx', args: ['mcp-server-fetch'], env: {} };
		for (const file of ['.claude.json', '.pi/agent/mcp.json', '.omp/agent/mcp.json']) {
			const read = JSON.parse(await sshOutput(destination, key, `cat ${file}`));
			expect(read.mcpServers, file).toEqual({ fetch });
		}
		expect(await sshOutput(destination, key, 'cat .codex/config.toml')).toContain(
			'[mcp_servers.fetch]\ncommand = "uvx"\nargs = ["mcp-server-fetch"]\n'
		);
	});

	await test.step('attach both to main, and give main a value of its own', async () => {
		await page
			.getByRole('navigation', { name: 'Main' })
			.getByRole('link', { name: 'Sandboxes' })
			.click();
		await page.getByRole('link', { name: /main/ }).click();
		const section = page.getByRole('list', { name: 'Attached to main' });
		const attach = page.getByRole('dialog', { name: 'Attach to main' });
		await page.getByRole('button', { name: 'Attach', exact: true }).click();
		await attach.getByRole('checkbox').first().click();
		await attach.getByRole('checkbox').last().click();
		await attach.getByRole('button', { name: 'Attach 2' }).click();
		await expect(attach).toBeHidden();
		await expect(section.getByRole('listitem')).toHaveCount(4);
		await expect(page.getByText('Restart to apply your setup changes')).toBeVisible();

		await page.getByRole('button', { name: 'Override for this sandbox: JOURNEY_TOKEN' }).click();
		const own = page.getByRole('dialog', { name: 'Override JOURNEY_TOKEN for main' });
		await own.getByLabel('Value', { exact: true }).fill(`${token}-main`);
		await own.getByRole('button', { name: 'Save override' }).click();
		await expect(own).toBeHidden();
		await expect(section.getByText('Override', { exact: true })).toBeVisible();
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
		expect(await sshOutput(destination, key, 'printenv JOURNEY_TOKEN')).toBe(`${token}-main`);
		expect(await sshOutput(destination, key, 'cat .journey/notes')).toBe(`token=${token}-main`);
		expect(await sshOutput(destination, key, 'stat -c %a .journey/notes')).toBe('600');
	});

	await test.step('remove the override, and detach the file, which the next start removes', async () => {
		const section = page.getByRole('list', { name: 'Attached to main' });
		const confirm = page.getByRole('alertdialog');
		await page.getByRole('button', { name: 'Detach JOURNEY_TOKEN' }).click();
		await confirm.getByRole('button', { name: 'Detach' }).click();
		await expect(confirm.getByRole('alert')).toContainText('~/.journey/notes');
		await confirm.getByRole('button', { name: 'Cancel' }).click();
		await page.getByRole('button', { name: 'Remove override of JOURNEY_TOKEN' }).click();
		await confirm.getByRole('button', { name: 'Remove override' }).click();
		await expect(section.getByText('Override', { exact: true })).toBeHidden();
		await page.getByRole('button', { name: 'Detach ~/.journey/notes' }).click();
		await confirm.getByRole('button', { name: 'Detach' }).click();
		await expect(section.getByRole('listitem')).toHaveCount(3);
		await expect(page.getByText('Restart to apply your setup changes')).toBeVisible();

		await page.getByRole('button', { name: 'Restart' }).click();
		const restarting = page.getByRole('button', { name: 'Restarting…' });
		await expect(restarting).toBeVisible();
		await expect(restarting).toBeHidden({ timeout: 60_000 });
		await expect(page.getByText('Running', { exact: true })).toBeVisible({ timeout: 90_000 });
		expect(await sshOutput(destination, key, 'test -e .journey/notes || echo gone')).toBe('gone');
		expect(await sshOutput(destination, key, 'printenv JOURNEY_TOKEN')).toBe(token);
	});

	await test.step('delete the sandbox', async () => {
		expect(containersOf('dora')).toHaveLength(1);
		await page.getByRole('button', { name: 'Delete sandbox' }).click();
		const dialog = page.getByRole('alertdialog', { name: 'Delete main?' });
		await expect(dialog).toContainText('remove it from Herdr');
		const confirm = dialog.getByRole('button', { name: 'Delete sandbox' });
		await expect(confirm).toBeDisabled();
		await dialog.getByLabel('Type main to confirm').fill('main');
		await confirm.click();
		await expect(page).toHaveURL('/sandboxes');
		await expect(page.getByText('Deleted main')).toBeVisible();
		await expect(page.getByText('No sandboxes yet')).toBeVisible();
		expect(containersOf('dora')).toEqual([]);
	});

	await test.step('delete the account', async () => {
		await page.getByRole('button', { name: 'Account' }).click();
		await page.getByRole('menuitem', { name: 'Account' }).click();
		await expect(page).toHaveURL('/account');
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
