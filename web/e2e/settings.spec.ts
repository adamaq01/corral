import { execFileSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { request, type APIRequestContext, type Locator, type Page } from '@playwright/test';
import type {
	AttachedEntry,
	Catalog,
	CatalogChange,
	CatalogSettings,
	OwnedSandbox,
	Sandbox,
	SharedEntry,
	SizeSetting
} from '../src/lib/api/generated/index.js';
import { expect, session, test } from './fixtures';
import { sshOutput } from './ssh';

type Size = Omit<SizeSetting, 'source' | 'enabled'>;

test.describe('an admin', () => {
	test.use({ role: 'admin' });

	test('shows the config entries locked, adds a size, and a member sees it offered', async ({
		page,
		baseURL
	}) => {
		const member = await request.newContext({ baseURL, storageState: session('member') });
		const offered = async () => (await catalog(member)).sizes.map((size) => size.key);

		await page.goto('/admin/settings/catalog');
		await expect(
			page.getByText(
				'What the config file sets is shown here and changed there; everything else is kept here.'
			)
		).toBeVisible();
		const images = page.getByRole('table', { name: 'Images' });
		const sizes = page.getByRole('table', { name: 'Sizes' });
		for (const locked of [
			entry(images, 'dev'),
			entry(images, 'dev-again'),
			entry(sizes, 'small'),
			entry(sizes, 'roomy')
		]) {
			await expect(locked).toContainText('Set in config');
			await expect(locked.getByRole('button')).toHaveCount(0);
			await expect(locked.getByRole('switch')).toHaveCount(0);
		}

		await addSize(page, {
			key: 'roomy-2',
			label: 'Roomy 2',
			cpuRequest: '500m',
			cpuLimit: '2',
			memoryRequest: '1Gi',
			memoryLimit: '3Gi',
			homeSize: '10Gi'
		});
		const roomy = entry(sizes, 'roomy-2');
		await expect(roomy).toContainText('Roomy 2');
		await expect(page.getByText('Unsaved changes')).toBeVisible();
		await save(page);
		await expect(page.getByText('Saved the catalog')).toBeVisible();
		await expect(page.getByText('Unsaved changes')).toBeHidden();
		expect(await offered()).toContain('roomy-2');

		await roomy.getByRole('switch', { name: 'Enable roomy-2' }).click();
		await save(page);
		await expect(page.getByText('Unsaved changes')).toBeHidden();
		await expect.poll(offered).not.toContain('roomy-2');
		await page.reload();
		await expect(roomy.getByRole('switch', { name: 'Enable roomy-2' })).not.toBeChecked();

		const sent: string[] = [];
		page.on('request', (sending) => {
			if (sending.method() !== 'GET') {
				sent.push(`${sending.method()} ${sending.url()}`);
			}
		});
		await page.getByRole('button', { name: 'Add size' }).click();
		const dialog = page.getByRole('dialog', { name: 'Add size' });
		await dialog.getByLabel('Key').fill('small');
		await expect(dialog.getByText('small is set in config')).toBeVisible();
		await expect(dialog.getByRole('button', { name: 'Add size' })).toBeDisabled();
		await dialog.getByRole('button', { name: 'Cancel' }).click();
		await expect(dialog).toBeHidden();
		expect(sent).toEqual([]);
		await member.dispose();
	});

	test('shows the built-in catalog as built in, and the first entries added replace it', async ({
		page
	}) => {
		const saved: CatalogChange[] = [];
		await page.route('**/api/admin/settings/catalog', async (route) => {
			if (route.request().method() === 'GET') {
				await route.fulfill({ json: builtin });
			} else {
				saved.push(route.request().postDataJSON());
				await route.fulfill({ status: 204 });
			}
		});
		await page.goto('/admin/settings/catalog');
		const images = page.getByRole('table', { name: 'Images' });
		const sizes = page.getByRole('table', { name: 'Sizes' });
		for (const offered of [entry(images, 'ubuntu'), entry(sizes, 'small')]) {
			await expect(offered).toContainText('Built in');
			await expect(offered).toContainText('Corral offers this until you add your own.');
			await expect(offered.getByRole('button')).toHaveCount(0);
			await expect(offered.getByRole('switch')).toHaveCount(0);
		}

		await page.getByRole('button', { name: 'Add image' }).click();
		const dialog = page.getByRole('dialog', { name: 'Add image' });
		await dialog.getByLabel('Key').fill('ubuntu');
		await dialog.getByLabel('Label').fill('Ubuntu');
		await dialog.getByLabel('Reference').fill('docker.io/library/ubuntu:24.04');
		await dialog.getByRole('button', { name: 'Add image' }).click();
		await expect(dialog).toBeHidden();
		const ubuntu = entry(images, 'ubuntu');
		await expect(ubuntu).toHaveCount(1);
		await expect(ubuntu).not.toContainText('Built in');
		await expect(ubuntu.getByRole('button', { name: 'Actions for ubuntu' })).toBeVisible();

		await addSize(page, {
			key: 'tiny',
			label: 'Tiny',
			cpuRequest: '250m',
			cpuLimit: '1',
			memoryRequest: '512Mi',
			memoryLimit: '1Gi',
			homeSize: '1Gi'
		});
		await expect(entry(sizes, 'small')).toHaveCount(0);
		await expect(sizes.getByRole('radio', { name: 'Default size: tiny' })).toBeChecked();
		await save(page);
		await expect.poll(() => saved.length).toBe(1);
		expect(saved[0]).toMatchObject({
			images: [{ key: 'ubuntu', reference: 'docker.io/library/ubuntu:24.04' }],
			sizes: [{ key: 'tiny' }],
			defaults: { image: 'ubuntu', size: 'tiny' }
		});
	});

	test('refuses removing a size a sandbox uses, naming it', async ({ page, baseURL }) => {
		test.setTimeout(180_000);
		const member = await newMember(page, baseURL!, 'gwen');
		await page.goto('/admin/settings/catalog');
		const sizes = page.getByRole('table', { name: 'Sizes' });
		await addSize(page, {
			key: 'snug',
			label: 'Snug',
			cpuRequest: '250m',
			cpuLimit: '1',
			memoryRequest: '512Mi',
			memoryLimit: '1Gi',
			homeSize: '1Gi'
		});
		await save(page);
		await expect(page.getByText('Saved the catalog')).toBeVisible();
		const created = await member.post('/api/sandboxes', { data: { name: 'main', size: 'snug' } });
		expect(created.status(), await created.text()).toBe(201);
		// Chromium fails loads in flight while Docker adds or removes a sandbox's host network
		// interface, so the test waits for the sandbox to run here and to be gone at the end.
		await expect
			.poll(async () => (await (await member.get('/api/sandboxes/main')).json()).state, {
				timeout: 90_000
			})
			.toBe('running');

		const snug = entry(sizes, 'snug');
		await snug.getByRole('button', { name: 'Actions for snug' }).click();
		await page.getByRole('menuitem', { name: 'Remove' }).click();
		const confirm = page.getByRole('alertdialog', { name: 'Remove snug?' });
		const remove = confirm.getByRole('button', { name: 'Remove size' });
		await expect(remove).toBeDisabled();
		await confirm.getByLabel('Type snug to confirm').fill('snug');
		await remove.click();
		await expect(confirm).toBeHidden();
		await expect(snug).toHaveCount(0);
		await save(page);
		const refused = page.getByRole('alert').filter({ hasText: 'snug' });
		await expect(refused).toContainText('main.gwen');

		await page.getByRole('button', { name: 'Reset' }).click();
		await expect(refused).toHaveCount(0);
		await expect(snug).toContainText('Snug');
		await expect(page.getByText('Unsaved changes')).toBeHidden();

		await snug.getByRole('switch', { name: 'Enable snug' }).click();
		await save(page);
		await expect
			.poll(async () => (await catalog(member)).sizes.map((size) => size.key))
			.not.toContain('snug');
		const deleted = await member.delete('/api/me');
		expect(deleted.ok(), await deleted.text()).toBe(true);
		await member.dispose();
		await expect
			.poll(
				async () => {
					const all: OwnedSandbox[] = await (await page.request.get('/api/admin/sandboxes')).json();
					return all.filter((sandbox) => sandbox.owner.handle === 'gwen').length;
				},
				{ timeout: 60_000 }
			)
			.toBe(0);
	});

	test('lowers the limit and shows a registry whose variable is not set', async ({
		page,
		baseURL
	}) => {
		const member = await newMember(page, baseURL!, 'hana');
		const maxSandboxes = async () => (await catalog(member)).maxSandboxes;

		await page.goto('/admin/settings/limits');
		const perMember = page.getByLabel('Sandboxes per member');
		await expect(perMember).toHaveValue('5');
		await expect(page.getByLabel('Running at once')).toHaveValue('6');
		await perMember.fill('2');
		await save(page);
		await expect(page.getByText('Saved the limits')).toBeVisible();
		expect(await maxSandboxes()).toBe(2);
		await perMember.fill('5');
		await save(page);
		await expect.poll(maxSandboxes).toBe(5);

		await page.goto('/admin/settings/registries');
		await page.getByRole('button', { name: 'Add registry' }).click();
		const dialog = page.getByRole('dialog', { name: 'Add registry' });
		await dialog.getByLabel('Registry').fill('ghcr.io');
		await dialog.getByLabel('Username').fill('corral');
		await dialog.getByLabel('Password variable').fill('CORRAL_GHCR_TOKEN');
		await dialog.getByRole('button', { name: 'Add registry' }).click();
		await expect(dialog).toBeHidden();
		await save(page);
		await expect(page.getByText('Saved the registries')).toBeVisible();
		const registry = entry(page.getByRole('table', { name: 'Registries' }), 'ghcr.io');
		await expect(registry).toContainText('CORRAL_GHCR_TOKEN');
		await expect(registry).toContainText('Variable not set');

		const deleted = await member.delete('/api/me');
		expect(deleted.ok(), await deleted.text()).toBe(true);
		await member.dispose();
	});

	test('shares a required and an offered variable, which reach a member and their sandbox', async ({
		page,
		baseURL,
		browser
	}) => {
		test.setTimeout(240_000);
		const member = await newMember(page, baseURL!, 'ines');
		const key = test.info().outputPath('id_ed25519');
		execFileSync('ssh-keygen', ['-q', '-t', 'ed25519', '-N', '', '-C', 'ines@laptop', '-f', key]);
		const added = await member.post('/api/keys', {
			data: { label: 'laptop', publicKey: readFileSync(`${key}.pub`, 'utf8') }
		});
		expect(added.ok(), await added.text()).toBe(true);
		const table = page.getByRole('table', { name: 'Shared setup' });
		const sandbox = async (): Promise<Sandbox> => (await member.get('/api/sandboxes/main')).json();

		await test.step('offer a variable that new sandboxes get', async () => {
			await page.goto('/admin/settings/shared-setup');
			await page.getByRole('button', { name: 'Add variable' }).click();
			const dialog = page.getByRole('dialog', { name: 'Add a shared variable' });
			await dialog.getByLabel('Name').fill('TEAM_PROXY');
			await dialog.getByLabel('Value').fill('http://proxy.lan:3128');
			await dialog.getByRole('radio', { name: 'Offered, members attach it' }).click();
			await expect(dialog.getByRole('switch', { name: 'Attach to new sandboxes' })).toBeChecked();
			await dialog.getByRole('button', { name: 'Save entry' }).click();
			await expect(dialog).toBeHidden();
			await expect(entry(table, 'TEAM_PROXY')).toContainText('Offered');
		});

		await test.step('a sandbox the member creates comes with it', async () => {
			const created = await member.post('/api/sandboxes', { data: { name: 'main' } });
			expect(created.status(), await created.text()).toBe(201);
			const attached: AttachedEntry[] = await (
				await member.get('/api/sandboxes/main/setup')
			).json();
			expect(attached.map(({ entry }) => [entry.target, entry.shared])).toEqual([
				['TEAM_PROXY', 'offered']
			]);
			await expect.poll(async () => (await sandbox()).state, { timeout: 90_000 }).toBe('running');
			await page.getByRole('button', { name: 'Refresh' }).click();
			await expect(entry(table, 'TEAM_PROXY').getByRole('cell').nth(3)).toHaveText('1');
		});

		await test.step('require a variable, which the member sees under Shared', async () => {
			await page.getByRole('button', { name: 'Add variable' }).click();
			const dialog = page.getByRole('dialog', { name: 'Add a shared variable' });
			await dialog.getByLabel('Name').fill('GATEWAY_URL');
			await dialog.getByLabel('Value').fill('https://llm.lan/v1');
			await expect(dialog.getByRole('radio', { name: 'Required for every sandbox' })).toBeChecked();
			await dialog.getByRole('button', { name: 'Save entry' }).click();
			await expect(dialog).toBeHidden();
			await expect(entry(table, 'GATEWAY_URL')).toContainText('Required');
			expect((await sandbox()).staleSetup).toBe(true);

			const context = await browser.newContext({
				baseURL,
				storageState: await member.storageState()
			});
			const setup = await context.newPage();
			await setup.goto('/setup');
			await expect(setup.getByRole('heading', { name: 'Shared' })).toBeVisible();
			const shared = setup.getByRole('list', { name: 'Shared' }).getByRole('listitem');
			await expect(shared.filter({ hasText: 'GATEWAY_URL' })).toContainText('Required');
			await expect(shared.filter({ hasText: 'TEAM_PROXY' })).toContainText('Offered');
			await context.close();
		});

		await test.step('restart, and the sandbox has both', async () => {
			const restarted = await member.post('/api/sandboxes/main/restart', {
				headers: { 'Content-Type': 'application/json' }
			});
			expect(restarted.ok(), await restarted.text()).toBe(true);
			await expect
				.poll(
					async () => {
						const { state, staleSetup } = await sandbox();
						return { state, staleSetup };
					},
					{ timeout: 90_000 }
				)
				.toEqual({ state: 'running', staleSetup: false });
			const { ssh } = await sandbox();
			const destination = ['-p', String(ssh.port), `${ssh.username}@${ssh.host}`];
			expect(await sshOutput(destination, key, 'printenv GATEWAY_URL')).toBe('https://llm.lan/v1');
			expect(await sshOutput(destination, key, 'printenv TEAM_PROXY')).toBe(
				'http://proxy.lan:3128'
			);
		});

		await test.step('delete both, each after typing its name', async () => {
			for (const name of ['GATEWAY_URL', 'TEAM_PROXY']) {
				await table.getByRole('button', { name: `Actions for ${name}` }).click();
				await page.getByRole('menuitem', { name: 'Delete…' }).click();
				const confirm = page.getByRole('alertdialog', { name: `Delete ${name}?` });
				const remove = confirm.getByRole('button', { name: 'Delete variable' });
				await expect(remove).toBeDisabled();
				await confirm.getByLabel(`Type ${name} to confirm`).fill(name);
				await remove.click();
				await expect(confirm).toBeHidden();
				await expect(entry(table, name)).toHaveCount(0);
			}
			const targets = (await sharedSetup(page)).map((shared) => shared.target);
			expect(targets).not.toContain('GATEWAY_URL');
			expect(targets).not.toContain('TEAM_PROXY');
		});

		const deleted = await member.delete('/api/me');
		expect(deleted.ok(), await deleted.text()).toBe(true);
		await member.dispose();
		await expect
			.poll(
				async () => {
					const all: OwnedSandbox[] = await (await page.request.get('/api/admin/sandboxes')).json();
					return all.filter((sandbox) => sandbox.owner.handle === 'ines').length;
				},
				{ timeout: 60_000 }
			)
			.toBe(0);
	});

	test('refuses requiring a name a member has, saying so under the choice', async ({
		page,
		baseURL
	}) => {
		const member = await newMember(page, baseURL!, 'jules');
		const own = await member.post('/api/setup', {
			data: {
				kind: 'variable',
				target: 'ANTHROPIC_BASE_URL',
				secret: false,
				applyToNew: true,
				value: { text: 'https://mine.example' }
			}
		});
		expect(own.status(), await own.text()).toBe(201);

		await page.goto('/admin/settings/shared-setup');
		await page.getByRole('button', { name: 'Add variable' }).click();
		const dialog = page.getByRole('dialog', { name: 'Add a shared variable' });
		await dialog.getByLabel('Name').fill('ANTHROPIC_BASE_URL');
		await dialog.getByLabel('Value').fill('https://llm.lan');
		await dialog.getByRole('button', { name: 'Save entry' }).click();
		const choice = dialog.getByRole('group', { name: 'Shared as' });
		await expect(choice.getByRole('alert')).toContainText(
			'variable ANTHROPIC_BASE_URL is also set by jules; ask them to rename theirs first'
		);
		const targets = async () => (await sharedSetup(page)).map((shared) => shared.target);
		expect(await targets()).not.toContain('ANTHROPIC_BASE_URL');

		await dialog.getByRole('radio', { name: 'Offered, members attach it' }).click();
		await expect(choice.getByRole('alert')).toHaveCount(0);
		await dialog.getByRole('button', { name: 'Save entry' }).click();
		await expect(dialog).toBeHidden();
		const table = page.getByRole('table', { name: 'Shared setup' });
		await expect(table.getByRole('columnheader').nth(1)).toHaveText('Shared as');
		const offered = entry(table, 'ANTHROPIC_BASE_URL');
		await expect(offered.getByRole('cell').nth(4)).toHaveText('jules');

		await offered.getByRole('button', { name: 'Actions for ANTHROPIC_BASE_URL' }).click();
		await page.getByRole('menuitem', { name: 'Edit' }).click();
		const editing = page.getByRole('dialog', { name: 'Edit ANTHROPIC_BASE_URL' });
		await expect(editing.getByLabel('Value')).toHaveValue('https://llm.lan');
		await expect(editing.getByRole('radio', { name: 'Offered, members attach it' })).toBeChecked();
		await editing.getByRole('radio', { name: 'Required for every sandbox' }).click();
		await editing.getByRole('button', { name: 'Save entry' }).click();
		await expect(
			editing.getByRole('group', { name: 'Shared as' }).getByRole('alert')
		).toContainText('is also set by jules');
		await editing.getByRole('button', { name: 'Cancel' }).click();
		await expect(editing).toBeHidden();
		await expect(offered.getByRole('cell').nth(1)).toHaveText('Offered');

		const { id } = (await sharedSetup(page)).find(
			(shared) => shared.target === 'ANTHROPIC_BASE_URL'
		)!;
		const removed = await page.request.delete(`/api/admin/settings/shared-setup/${id}`);
		expect(removed.ok(), await removed.text()).toBe(true);
		const deleted = await member.delete('/api/me');
		expect(deleted.ok(), await deleted.text()).toBe(true);
		await member.dispose();
	});
});

const builtin: CatalogSettings = {
	images: [
		{
			key: 'ubuntu',
			label: 'Ubuntu 24.04',
			reference: 'ghcr.io/adamaq01/corral-sandbox',
			description: '',
			enabled: true,
			source: 'builtin'
		}
	],
	sizes: [
		{
			key: 'small',
			label: 'Small: 2 CPUs, 4 GiB',
			cpuRequest: '500m',
			cpuLimit: '2',
			memoryRequest: '1Gi',
			memoryLimit: '4Gi',
			homeSize: '10Gi',
			enabled: true,
			source: 'builtin'
		}
	],
	namedImages: null,
	namedImagesSource: 'database',
	defaultImage: 'ubuntu',
	defaultSize: 'small',
	defaultsSource: 'database',
	defaultsFellBack: false,
	shadowedImages: [],
	shadowedSizes: []
};

/** The row of `table` whose key, or registry, is exactly `key`. */
function entry(table: Locator, key: string): Locator {
	return table.getByRole('row').filter({ has: table.page().getByText(key, { exact: true }) });
}

async function save(page: Page) {
	await page.getByRole('button', { name: 'Save', exact: true }).click();
}

async function addSize(page: Page, size: Size) {
	await page.getByRole('button', { name: 'Add size' }).click();
	const dialog = page.getByRole('dialog', { name: 'Add size' });
	await dialog.getByLabel('Key').fill(size.key);
	await dialog.getByLabel('Label').fill(size.label);
	await dialog.getByLabel('CPU request').fill(size.cpuRequest);
	await dialog.getByLabel('CPU limit').fill(size.cpuLimit);
	await dialog.getByLabel('Memory request').fill(size.memoryRequest);
	await dialog.getByLabel('Memory limit').fill(size.memoryLimit);
	await dialog.getByLabel('Home size').fill(size.homeSize);
	await dialog.getByRole('button', { name: 'Add size' }).click();
	await expect(dialog).toBeHidden();
}

async function catalog(member: APIRequestContext): Promise<Catalog> {
	const response = await member.get('/api/catalog');
	expect(response.ok(), await response.text()).toBe(true);
	return response.json();
}

async function sharedSetup(page: Page): Promise<SharedEntry[]> {
	const response = await page.request.get('/api/admin/settings/shared-setup');
	expect(response.ok(), await response.text()).toBe(true);
	return response.json();
}

/** A member signed in with a local account made for one test, which deletes it with `/api/me`. */
async function newMember(page: Page, baseURL: string, username: string) {
	const account = { username, password: randomBytes(18).toString('base64') };
	const made = await page.request.post('/auth/local/accounts', {
		data: { ...account, role: 'member' }
	});
	expect(made.ok(), await made.text()).toBe(true);
	const member = await request.newContext({ baseURL });
	const signedIn = await member.post('/auth/local/login', { data: account });
	expect(signedIn.ok(), await signedIn.text()).toBe(true);
	return member;
}
