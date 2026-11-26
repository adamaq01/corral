import { randomBytes } from 'node:crypto';
import { request, type APIRequestContext, type Locator, type Page } from '@playwright/test';
import type {
	Catalog,
	CatalogChange,
	CatalogSettings,
	OwnedSandbox,
	SizeSetting
} from '../src/lib/api/generated/index.js';
import { expect, session, test } from './fixtures';

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
