import type { Page } from '@playwright/test';
import type {
	EngineEvent,
	Fact,
	LocalAccount,
	OwnedSandbox,
	SandboxDetails,
	SandboxStatus,
	User
} from '../src/lib/api/generated/index.js';
import { expect, test, idle } from './fixtures';
import { inEveryView } from './views';

const days = (n: number) => new Date(Date.UTC(2026, 10, 22 - n, 9, 30)).toISOString();

const users: User[] = [
	user('ada', 'admin', 'active', 'ada@example.com', 40),
	user('bob', 'member', 'active', 'bob.the.builder@example.com', 31),
	user('grace-hopper', 'member', 'disabled', null, 12),
	user('linus', 'member', 'deleting', 'linus@example.com', 3)
];

const accounts: LocalAccount[] = [
	{ username: 'ada', role: 'admin', createdAt: days(40), yours: true },
	{ username: 'bob', role: 'member', createdAt: days(31), yours: false },
	{ username: 'grace-hopper', role: 'member', createdAt: days(12), yours: false }
];

const sandboxes: OwnedSandbox[] = [
	sandbox('ada', 'main', 'dev', 'small', true, { state: 'running' }, 39),
	{
		...sandbox('ada', 'scratch', 'dev', 'small', false, { state: 'sleeping' }, 20),
		deleteAt: '2026-11-24T18:40:00Z'
	},
	{
		...sandbox('bob', 'main', 'dev', 'small', true, { state: 'starting' }, 1),
		staleEnvironment: true
	},
	sandbox(
		'grace-hopper',
		'compiler',
		'dev',
		'small',
		true,
		{ state: 'failed', detail: 'the image could not be pulled' },
		10
	),
	sandbox('linus', 'kernel', 'dev', 'small', true, { state: 'deleting' }, 3)
];

const id = 'sbx-4f2a9c1e7b3d';
const at = (minutes: number) => new Date(Date.UTC(2026, 10, 22, 10, minutes, 12)).toISOString();

const failing: SandboxDetails = {
	sandbox: sandbox(
		'grace-hopper',
		'compiler',
		'dev',
		'small',
		true,
		{ state: 'failed', detail: 'Pod is not ready' },
		10
	),
	facts: [
		fact('Namespace', 'corral-sandboxes'),
		fact('Sandbox', id),
		fact('Sandbox ready', 'False: PodFailed, Pod is not ready'),
		fact('Pod', id),
		fact('Node', 'homelab-worker-2'),
		fact('Phase', 'Running'),
		fact('Pod started', '2026-11-22 10:02:41 UTC'),
		fact('Restarts', '4'),
		fact(
			'Container state',
			'Waiting: CrashLoopBackOff, back-off 1m20s restarting failed container'
		),
		fact('Last termination', 'Terminated: OOMKilled, exit code 137, at 2026-11-22 10:11:05 UTC'),
		fact('Image', 'ghcr.io/adamaq01/corral-sandbox:1.4'),
		fact('Image digest', 'sha256:9b1c4e0d8f2a7c3b6e5d4f1a0b9c8d7e6f5a4b3c2d1e0f9a8b7c6d5e4f3a2b1c'),
		fact('Home volume claim', `home-${id}`),
		fact('Home volume size', '20Gi'),
		fact('Storage class', 'longhorn'),
		fact('Home volume', 'pvc-0c6b2f9e-3d4a-4e8b-9f1c-7a2d5e8b1c4f'),
		fact('Home volume phase', 'Bound'),
		fact('Condition Ready', 'False: ContainersNotReady, containers with unready status: [sandbox]'),
		fact('Condition PodScheduled', 'True')
	],
	events: [
		event(
			12,
			'warning',
			`Pod ${id}`,
			'BackOff',
			'Back-off restarting failed container sandbox (9 times)'
		),
		event(
			11,
			'normal',
			`Pod ${id}`,
			'Pulled',
			'Container image "ghcr.io/adamaq01/corral-sandbox:1.4" already present on machine'
		),
		event(
			11,
			'warning',
			`Pod ${id}`,
			'OOMKilling',
			'Memory cgroup out of memory: killed process 4127 (cc1plus)'
		),
		event(3, 'normal', `Pod ${id}`, 'Started', 'Started container sandbox'),
		event(
			2,
			'normal',
			`Pod ${id}`,
			'Scheduled',
			`Successfully assigned corral-sandboxes/${id} to homelab-worker-2`
		),
		event(
			1,
			'normal',
			`PersistentVolumeClaim home-${id}`,
			'ProvisioningSucceeded',
			`Successfully provisioned volume pvc-0c6b2f9e-3d4a-4e8b-9f1c-7a2d5e8b1c4f`
		)
	]
};

const sleeping: SandboxDetails = {
	sandbox: sandboxes[1],
	facts: [
		fact('Container', 'corral-sbx-7d0e3b5a91c2'),
		fact('Container ID', '5f0d2c8e1b7a4f3e9c6d0b2a8e1f7c4d3b9a6e2f0c8d1b5a7e4f3c2d9b0a6e1f'),
		fact('State', 'exited'),
		fact('Started', '2026-11-21 09:12:40 UTC'),
		fact('Finished', '2026-11-21 18:40:09 UTC'),
		fact('Exit code', '143'),
		fact('Restarts', '0'),
		fact('Network', 'corral-sandboxes'),
		fact('Image', 'corral-sandbox:dev'),
		fact('Home volume', 'corral-home-sbx-7d0e3b5a91c2'),
		fact('Volume driver', 'local')
	],
	events: []
};

function fact(label: string, value: string | null): Fact {
	return { label, value };
}

function event(
	minute: number,
	kind: EngineEvent['kind'],
	object: string,
	reason: string,
	message: string
): EngineEvent {
	return { at: at(minute), kind, object, reason, message };
}

/** The API answering `path` with `body`, so each screenshot shows the same rows. */
async function answer(page: Page, path: string, body: unknown, status = 200) {
	await page.route(path, (route) =>
		route.request().method() === 'GET' ? route.fulfill({ status, json: body }) : route.fallback()
	);
}

inEveryView((look) => {
	test.use({ role: 'admin' });

	test('users', async ({ page }) => {
		await answer(page, '/api/admin/users', users);
		await page.goto('/admin/users');
		await page.getByRole('row', { name: /linus/ }).waitFor();
		await look(page, 'admin-users');

		await page.getByRole('button', { name: 'Actions for bob' }).click();
		await page.getByRole('menuitem', { name: 'Delete user' }).click();
		await page.getByRole('alertdialog').waitFor();
		await look(page, 'admin-users-delete');

		await page.keyboard.press('Escape');
		await page.getByRole('alertdialog').waitFor({ state: 'hidden' });
		await page.getByRole('button', { name: 'Actions for bob' }).click();
		await page.getByRole('menuitem', { name: 'Disable user' }).click();
		await page.getByRole('alertdialog').waitFor();
		await look(page, 'admin-users-disable');
	});

	test('users, empty', async ({ page }) => {
		await answer(page, '/api/admin/users', []);
		await page.goto('/admin/users');
		await page.getByText('No users yet').waitFor();
		await look(page, 'admin-users-empty');
	});

	test('local accounts', async ({ page }) => {
		await answer(page, '/auth/local/accounts', accounts);
		await page.goto('/admin/accounts');
		await page.getByRole('row', { name: /grace-hopper/ }).waitFor();
		await look(page, 'admin-accounts');

		await page.getByRole('button', { name: 'New account' }).click();
		await page.getByRole('dialog').waitFor();
		await look(page, 'admin-accounts-new');
	});

	test('local accounts, empty', async ({ page }) => {
		await answer(page, '/auth/local/accounts', []);
		await page.goto('/admin/accounts');
		await page.getByText('No local accounts').waitFor();
		await look(page, 'admin-accounts-empty');
	});

	test('all sandboxes', async ({ page }) => {
		await answer(page, '/api/admin/sandboxes', sandboxes);
		await page.goto('/admin/sandboxes');
		await page.getByRole('row', { name: /kernel/ }).waitFor();
		const detail = page.getByText('the image could not be pulled');
		await expect(detail).toBeVisible();
		expect(await detail.evaluate((text) => text.scrollWidth <= text.clientWidth)).toBe(true);
		await look(page, 'admin-sandboxes');
	});

	test('all sandboxes, empty', async ({ page }) => {
		await answer(page, '/api/admin/sandboxes', []);
		await page.goto('/admin/sandboxes');
		await page.getByText('No sandboxes yet').waitFor();
		await look(page, 'admin-sandboxes-empty');
	});

	test('sandbox details', async ({ page }) => {
		await answer(page, '/api/admin/sandboxes', sandboxes);
		await answer(page, '/api/admin/sandboxes/grace-hopper/compiler', failing);
		await page.goto('/admin/sandboxes');
		await page.getByRole('link', { name: 'compiler.grace-hopper details' }).click();
		await expect(page).toHaveURL('/admin/sandboxes/grace-hopper/compiler');
		await page.getByText('OOMKilling').waitFor();
		await look(page, 'admin-sandbox');
	});

	test('sandbox details, asleep without events', async ({ page }) => {
		await answer(page, '/api/admin/sandboxes/ada/scratch', sleeping);
		await page.goto('/admin/sandboxes/ada/scratch');
		await page.getByText('No recent events').waitFor();
		await look(page, 'admin-sandbox-asleep');
	});

	test('sandbox details, loading, missing and failed', async ({ page }) => {
		const path = '/api/admin/sandboxes/ada/scratch';
		await page.route(path, () => {});
		await page.goto('/admin/sandboxes/ada/scratch');
		await page.getByRole('status', { name: 'Loading the sandbox' }).waitFor();
		await look(page, 'admin-sandbox-loading');

		await page.unrouteAll();
		await answer(page, path, { error: 'sandbox scratch.ada not found' }, 404);
		await page.reload();
		await page.getByText('ada has no sandbox named scratch').waitFor();
		await look(page, 'admin-sandbox-missing');

		await page.unrouteAll();
		await answer(page, path, { error: 'internal error' }, 500);
		await page.reload();
		await page.getByText('Loading the sandbox failed').waitFor();
		await look(page, 'admin-sandbox-failed');
	});

	test('all sandboxes, failed', async ({ page }) => {
		await answer(page, '/api/admin/sandboxes', { error: 'internal error' }, 500);
		await page.goto('/admin/sandboxes');
		await page.getByRole('button', { name: 'Retry' }).waitFor();
		await look(page, 'admin-sandboxes-failed');
	});
});

function user(
	handle: string,
	role: User['role'],
	status: User['status'],
	email: string | null,
	age: number
): User {
	return {
		id: crypto.randomUUID(),
		handle,
		email,
		picture: null,
		role,
		status,
		createdAt: days(age),
		provider: 'oidc',
		allowance: null
	};
}

function sandbox(
	owner: string,
	name: string,
	image: string,
	size: string,
	isDefault: boolean,
	status: SandboxStatus,
	age: number
): OwnedSandbox {
	return {
		owner: { handle: owner, picture: null },
		name,
		image,
		size,
		isDefault,
		ssh: { username: `${name}.${owner}`, host: 'corral.example.com', port: 2222 },
		createdAt: days(age),
		staleEnvironment: false,
		idle,
		deleteAt: null,
		...status
	};
}
