import type { Catalog, Sandbox } from '../src/lib/api/generated/index.js';
import { account, expect, signIn, test } from './fixtures';

// A LAN hostname over plain HTTP is not a secure context, so the browser withholds the Clipboard
// API there, unlike on 127.0.0.1. Chromium would hand the name to a proxy from the environment.
const lan = 'corral.lan';

test.use({
	launchOptions: {
		args: [`--host-resolver-rules=MAP ${lan} 127.0.0.1`, '--proxy-server=direct://']
	}
});

const sandbox: Sandbox = {
	name: 'main',
	image: 'dev',
	size: 'small',
	isDefault: true,
	ssh: { username: 'bob+main', host: lan, port: 2222 },
	createdAt: '2026-11-02T09:30:00Z',
	state: 'running'
};

const catalog: Catalog = {
	images: [{ key: 'dev', label: 'Development' }],
	sizes: [{ key: 'small', label: 'Small' }],
	defaultImage: 'dev',
	defaultSize: 'small',
	maxSandboxes: 8,
	ssh: { host: lan, port: 2222 }
};

test('copy buttons work on a plain HTTP origin', async ({ page, context, baseURL }) => {
	const trusted = new URL(baseURL!);
	const plain = `http://${lan}:${trusted.port}`;

	await page.goto(`${plain}/sign-in`);
	expect(await page.evaluate(() => window.isSecureContext)).toBe(false);
	await signIn(page, account('member'));
	await expect(page).toHaveURL(`${plain}/sandboxes`);

	await page.route('**/api/sandboxes/main', (route) => route.fulfill({ json: sandbox }));
	await page.route('**/api/catalog', (route) => route.fulfill({ json: catalog }));
	await page.goto(`${plain}/sandboxes/main`);
	await page.getByRole('button', { name: 'Copy SSH config' }).click();
	await expect(page.getByRole('figure', { name: 'SSH config' })).toContainText('Copied');

	await context.grantPermissions(['clipboard-read'], { origin: trusted.origin });
	const reader = await context.newPage();
	await reader.goto(`${trusted.origin}/sign-in`);
	expect(await reader.evaluate(() => navigator.clipboard.readText())).toBe(
		['Host corral-main', `  HostName ${lan}`, '  Port 2222', '  User bob+main'].join('\n')
	);
});
