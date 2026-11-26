import type { Page } from '@playwright/test';
import type { CreateSandbox, SetupEntry } from '../src/lib/api/generated/index.js';
import { expect, test } from './fixtures';

test.use({ role: 'member' });

const setAt = '2026-11-20T17:45:00Z';

function entry(id: string, target: string, extra: Partial<SetupEntry> = {}): SetupEntry {
	return {
		id,
		kind: 'variable',
		target,
		secret: false,
		template: false,
		executable: false,
		applyToNew: false,
		value: { setAt, size: 4, shown: 'text', text: 'text' },
		createdAt: setAt,
		sandboxes: [],
		references: [],
		duplicate: false,
		shared: null,
		shadows: false,
		shadowed: false,
		...extra
	};
}

/** Opens the New sandbox dialog over `entries`, keeping each body sent and sending nothing on. */
async function openDialog(page: Page, entries: SetupEntry[]) {
	const sent: CreateSandbox[] = [];
	await page.route('**/api/setup', (route) => route.fulfill({ json: entries }));
	await page.route('**/api/sandboxes', (route) => {
		if (route.request().method() === 'GET') {
			return route.fulfill({ json: [] });
		}
		sent.push(route.request().postDataJSON());
		return route.abort();
	});
	await page.goto('/sandboxes');
	await page.getByRole('button', { name: 'New sandbox' }).first().click();
	const dialog = page.getByRole('dialog', { name: 'New sandbox' });
	return { dialog, sent, create: dialog.getByRole('button', { name: 'Create sandbox' }) };
}

test('a sandbox name outside the rule is refused before anything is sent', async ({ page }) => {
	const { dialog, sent, create } = await openDialog(page, []);
	const name = dialog.getByLabel('Name');

	await name.fill('Walk 1');
	await expect(dialog.getByRole('alert')).toHaveText(
		'Up to 32 lowercase letters, digits and dashes.'
	);
	await expect(name).toHaveAttribute('aria-invalid', 'true');
	await expect(create).toBeDisabled();
	await name.press('Enter');
	await expect(dialog).toBeVisible();

	await name.fill('walk-1');
	await expect(dialog.getByRole('alert')).toHaveCount(0);
	await expect(dialog).toContainText('ssh -p');
	await expect(create).toBeEnabled();
	expect(sent).toEqual([]);
});

test('the dialog lists what the sandbox comes with and sends the choice and the repository', async ({
	page
}) => {
	const { dialog, sent, create } = await openDialog(page, [
		entry('gateway', 'GATEWAY_URL', { shared: 'required', applyToNew: true }),
		entry('proxy', 'TEAM_PROXY', { shared: 'offered', applyToNew: true }),
		entry('token', 'GITHUB_TOKEN', { secret: true })
	]);
	const setup = dialog.getByRole('list', { name: 'What it comes with' });
	const required = setup.getByRole('listitem').filter({ hasText: 'GATEWAY_URL' });
	await expect(required).toContainText('Required');
	await expect(required.getByRole('checkbox')).toHaveCount(0);
	await expect(setup.getByRole('checkbox')).toHaveCount(2);
	const proxy = setup.getByRole('checkbox', { name: 'TEAM_PROXY' });
	const token = setup.getByRole('checkbox', { name: 'GITHUB_TOKEN' });
	await expect(proxy).toBeChecked();
	await expect(setup.getByRole('listitem').filter({ hasText: 'TEAM_PROXY' })).toContainText(
		'Offered'
	);
	await expect(token).not.toBeChecked();
	await expect(setup.getByRole('listitem').filter({ hasText: 'GITHUB_TOKEN' })).toContainText(
		'Secret'
	);
	await expect(dialog).not.toContainText('the shared one');

	await proxy.click();
	await token.click();
	await expect(proxy).not.toBeChecked();
	await expect(token).toBeChecked();

	await dialog.getByLabel('Name').fill('walk');
	const repository = dialog.getByRole('group', { name: 'Repository' });
	await expect(repository).toContainText('Cloned into ~/<name> at the first start');
	await expect(repository.getByLabel('Branch')).toBeDisabled();
	await repository.getByLabel('Clone URL').fill('git@github.com:me/walk.git');
	await expect(repository).toContainText('Cloned into ~/walk at the first start');
	await repository.getByLabel('Branch').fill('feat/setup');
	await create.click();

	await expect.poll(() => sent).toHaveLength(1);
	expect(sent[0]).toMatchObject({
		name: 'walk',
		setup: ['token'],
		repository: { url: 'git@github.com:me/walk.git', branch: 'feat/setup' }
	});
});

test('a repository URL with a password, or a bad branch, is refused before anything is sent', async ({
	page
}) => {
	const { dialog, sent, create } = await openDialog(page, []);
	await expect(dialog.getByRole('group', { name: 'Setup' })).toContainText(
		'No variables or files yet'
	);
	const url = dialog.getByLabel('Clone URL');
	const branch = dialog.getByLabel('Branch');
	const rule = 'An https://, ssh:// or git@host:path address, with no password in it.';
	await expect(dialog.getByText(rule)).toBeVisible();
	await expect(dialog.getByRole('alert')).toHaveCount(0);

	await url.fill('https://me:tok3n@github.com/me/x.git');
	await expect(dialog.getByRole('alert')).toHaveText(rule);
	await expect(url).toHaveAttribute('aria-invalid', 'true');
	await expect(create).toBeDisabled();
	await url.press('Enter');
	await expect(dialog).toBeVisible();

	await url.fill('https://github.com/me/x.git');
	await expect(dialog.getByRole('alert')).toHaveCount(0);
	await expect(create).toBeEnabled();

	await branch.fill('a..b');
	await expect(dialog.getByRole('alert')).toContainText('git branch name');
	await expect(branch).toHaveAttribute('aria-invalid', 'true');
	await expect(create).toBeDisabled();
	await branch.fill('main');
	await expect(dialog.getByRole('alert')).toHaveCount(0);
	await expect(create).toBeEnabled();
	expect(sent).toEqual([]);
});

test('an offered entry the member replaces with their own is left out, saying so', async ({
	page
}) => {
	const { dialog } = await openDialog(page, [
		entry('shared-editor', 'EDITOR', { shared: 'offered', applyToNew: true, shadowed: true }),
		entry('own-editor', 'EDITOR', { applyToNew: true, shadows: true })
	]);
	const setup = dialog.getByRole('list', { name: 'What it comes with' });
	await expect(setup.getByRole('checkbox')).toHaveCount(1);
	await expect(setup.getByRole('checkbox', { name: 'EDITOR' })).toBeChecked();
	await expect(setup).not.toContainText('Offered');
	await expect(dialog.getByText('Your EDITOR takes the place of the shared one.')).toBeVisible();
});
