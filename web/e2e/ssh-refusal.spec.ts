import { execFileSync, spawnSync } from 'node:child_process';
import {
	chmodSync,
	existsSync,
	mkdirSync,
	mkdtempSync,
	readFileSync,
	rmSync,
	writeFileSync
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import type { Catalog, OpksshLogin, Provider, Sandbox } from '../src/lib/api/generated/index.js';
import { expect, test } from './fixtures';

const home = mkdtempSync(join(tmpdir(), 'corral-web-refusal-'));
test.afterAll(() => rmSync(home, { recursive: true, force: true }));

test.use({ role: 'member' });

/**
 * Runs ssh as a person would, with the config blocks the sandbox page shows, a key Corral does
 * not know, and an askpass program that records each prompt ssh would have shown.
 */
function login(config: string, args: string[]) {
	const file = join(home, 'config');
	writeFileSync(file, config);
	const prompts = join(home, 'prompts');
	rmSync(prompts, { force: true });
	const ssh = spawnSync(
		'ssh',
		[
			'-F',
			file,
			'-i',
			join(home, 'id_ed25519'),
			'-o',
			'IdentitiesOnly=yes',
			'-o',
			'IdentityAgent=none',
			'-o',
			`UserKnownHostsFile=${process.env.CORRAL_E2E_KNOWN_HOSTS}`,
			'-o',
			'StrictHostKeyChecking=yes',
			'-o',
			'ConnectTimeout=10',
			'-o',
			'NumberOfPasswordPrompts=1',
			...args,
			'true'
		],
		{
			encoding: 'utf8',
			stdio: ['ignore', 'pipe', 'pipe'],
			timeout: 60_000,
			env: {
				PATH: process.env.PATH,
				HOME: home,
				SSH_ASKPASS: join(home, 'askpass'),
				SSH_ASKPASS_REQUIRE: 'force'
			}
		}
	);
	return {
		status: ssh.status,
		stderr: ssh.stderr,
		prompts: existsSync(prompts) ? readFileSync(prompts, 'utf8').trim().split('\n') : []
	};
}

test('a refused key fails at once, with no password prompt, under either config block', async ({
	page
}) => {
	const [, port] = /^\[127\.0\.0\.1\]:(\d+) /.exec(
		readFileSync(process.env.CORRAL_E2E_KNOWN_HOSTS!, 'utf8')
	)!;
	const sandbox: Sandbox = {
		name: 'main',
		image: 'dev',
		size: 'small',
		isDefault: true,
		ssh: { username: 'main.bob', host: '127.0.0.1', port: Number(port) },
		createdAt: '2026-11-02T09:30:00Z',
		staleCredentials: false,
		state: 'running'
	};
	const catalog: Catalog = {
		images: [{ key: 'dev', label: 'Development' }],
		sizes: [{ key: 'small', label: 'Small' }],
		defaultImage: 'dev',
		defaultSize: 'small',
		maxSandboxes: 8,
		ssh: { host: '127.0.0.1', port: Number(port) }
	};
	const providers: Provider[] = [
		{
			id: 'sso',
			label: 'Single sign-on',
			kind: 'redirect',
			login: '/auth/sso/login',
			accounts: null,
			opkssh: '/auth/sso/opkssh'
		}
	];
	const opkssh: OpksshLogin = {
		issuer: 'https://sso.example.lan',
		clientId: 'opkssh',
		lifetimeHours: 24
	};
	await page.route('**/api/sandboxes/main', (route) => route.fulfill({ json: sandbox }));
	await page.route('**/api/catalog', (route) => route.fulfill({ json: catalog }));
	await page.route('**/auth/providers', (route) => route.fulfill({ json: providers }));
	await page.route('**/auth/sso/opkssh', (route) => route.fulfill({ json: opkssh }));
	await page.goto('/sandboxes/main');
	const snippet = (label: string) =>
		page.getByRole('figure', { name: label, exact: true }).locator('code').innerText();
	const config = [
		await snippet('SSH config'),
		await snippet('SSH config (end of ~/.ssh/config)'),
		''
	].join('\n');

	execFileSync('ssh-keygen', ['-q', '-t', 'ed25519', '-N', '', '-f', join(home, 'id_ed25519')]);
	writeFileSync(
		join(home, 'askpass'),
		`#!/bin/sh\necho "$1" >> '${join(home, 'prompts')}'\necho wrong\n`
	);
	chmodSync(join(home, 'askpass'), 0o755);
	// A fresh certificate, as `opkssh login` leaves it, so the config's renewal hook stays idle.
	mkdirSync(join(home, '.ssh'));
	writeFileSync(join(home, '.ssh', 'corral-opkssh-cert.pub'), '');

	const unknownSandbox = ['-p', port, 'nosuch.bob@127.0.0.1'];
	for (const [refused, args] of [
		['the sandbox alias', ['corral-main']],
		['the opkssh block alone', unknownSandbox]
	] as const) {
		const attempt = login(config, [...args]);
		expect(attempt.prompts, refused).toEqual([]);
		expect(attempt.status, `${refused}: ${attempt.stderr}`).toBe(255);
		expect(attempt.stderr, refused).toContain('Permission denied (');
	}

	const prompting = login(config.replaceAll('  PreferredAuthentications publickey\n', ''), [
		...unknownSandbox
	]);
	expect(prompting.prompts.length, prompting.stderr).toBeGreaterThan(0);
});
