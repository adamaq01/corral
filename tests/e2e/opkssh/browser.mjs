// The browser steps of tests/e2e/opkssh.sh, with the Playwright that web/ installs.
//
//   reset <url> <password>                 sets VoidAuth's admin password from its reset link
//   sign-in <base> <profile> <user> <password> <dir>
//                                          signs in to Corral through VoidAuth, creates sandbox
//                                          main, and saves its page's opkssh snippets in <dir>
//   no-card <base> <user> <password>       a local account's sandbox page offers no opkssh
//   approve <url> <profile>                completes an `opkssh login` with the profile's session
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { chromium } from '../../../web/node_modules/playwright/index.mjs';

const [command, ...args] = process.argv.slice(2);
// VoidAuth serves the test CA's certificate, which only opkssh and Corral are told to trust.
const options = { ignoreHTTPSErrors: true };

async function voidauthLogin(page, username, password) {
	await page.getByLabel(/username or email/i).fill(username);
	await page.locator('input[type=password]').first().fill(password);
	await page.getByRole('checkbox', { name: /remember me/i }).check();
	await page.locator('input[type=password]').first().press('Enter');
	// VoidAuth offers to register a passkey after a password login.
	const skip = page.locator('mat-dialog-container').getByRole('button', { name: /skip/i });
	await skip.click({ timeout: 10_000 }).catch(() => {});
}

async function reset([url, password]) {
	const browser = await chromium.launch();
	const page = await browser.newPage(options);
	await page.goto(url);
	for (const field of await page.locator('input[type=password]').all()) {
		await field.fill(password);
	}
	await page.getByRole('button', { name: /reset password/i }).click();
	await page.waitForURL(/\/login/);
	await browser.close();
}

async function snippet(page, label) {
	return page.locator('figure', { hasText: label }).locator('code').innerText();
}

async function signIn([base, profile, username, password, dir]) {
	const context = await chromium.launchPersistentContext(profile, options);
	const page = context.pages()[0] ?? (await context.newPage());
	await page.goto(`${base}/auth/oidc/start?next=/sandboxes`);
	await voidauthLogin(page, username, password);
	await page.waitForURL(`${base}/sandboxes`);
	const created = await page.request.post(`${base}/api/sandboxes`, { data: { name: 'main' } });
	if (!created.ok()) {
		throw new Error(`creating sandbox main: ${created.status()} ${await created.text()}`);
	}
	await page.goto(`${base}/sandboxes/main`);
	await page.getByText('Connect with opkssh', { exact: true }).waitFor();
	writeFileSync(join(dir, 'opk-config.yml'), `${await snippet(page, 'opkssh config')}\n`);
	writeFileSync(join(dir, 'ssh-hook.conf'), `${await snippet(page, 'SSH config (end')}\n`);
	await context.close();
}

async function noCard([base, username, password]) {
	const browser = await chromium.launch();
	const page = await browser.newPage(options);
	const signedIn = await page.request.post(`${base}/auth/local/login`, {
		data: { username, password }
	});
	if (!signedIn.ok()) {
		throw new Error(`signing in as ${username}: ${signedIn.status()}`);
	}
	await page.request.post(`${base}/api/sandboxes`, { data: { name: 'main' } });
	await page.goto(`${base}/sandboxes/main`);
	await page.getByRole('button', { name: 'Copy SSH config' }).waitFor();
	await page.waitForLoadState('networkidle');
	const cards = await page.getByText('Connect with opkssh', { exact: true }).count();
	await browser.close();
	if (cards !== 0) {
		throw new Error(`${username} was offered opkssh`);
	}
}

async function approve([url, profile]) {
	const context = await chromium.launchPersistentContext(profile, options);
	const page = context.pages()[0] ?? (await context.newPage());
	try {
		await page.goto(url);
		await page.waitForURL(/\/login-callback/, { timeout: 10_000 });
		console.log(`approved ${url}`);
	} finally {
		await context.close();
	}
}

const commands = { reset, 'sign-in': signIn, 'no-card': noCard, approve };
await commands[command](args);
