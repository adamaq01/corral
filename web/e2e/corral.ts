import { execFileSync, spawn, type ChildProcess } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import {
	copyFileSync,
	mkdirSync,
	mkdtempSync,
	openSync,
	readFileSync,
	rmSync,
	writeFileSync
} from 'node:fs';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';
import { request } from '@playwright/test';
import { session, type Account, type Role } from './fixtures';

const web = resolve(import.meta.dirname, '..');
const repo = resolve(web, '..');

/**
 * Builds the UI and runs the real corral-server on it: SQLite in a temporary directory, local
 * accounts made with its CLI, and the Docker engine on a network of its own. Tests reach it at
 * `CORRAL_E2E_URL` and sign in with the accounts in `CORRAL_E2E_ACCOUNTS`.
 */
export default async function corral(): Promise<() => Promise<void>> {
	const run = new Run();
	try {
		await run.start();
	} catch (error) {
		await run.stop();
		throw error;
	}
	return () => run.stop();
}

class Run {
	readonly dir = mkdtempSync(join(tmpdir(), 'corral-web-e2e-'));
	readonly network = `corral-web-e2e-${randomBytes(4).toString('hex')}`;
	networkCreated = false;
	server: ChildProcess | null = null;

	get log(): string {
		return join(this.dir, 'corral-server.log');
	}

	async start() {
		execFileSync('npm', ['run', 'build'], { cwd: web, stdio: 'inherit' });
		const binary = serverBinary();
		const [publicPort, internalPort] = [await freePort(), await freePort()];
		const url = `http://127.0.0.1:${publicPort}`;

		execFileSync('docker', ['network', 'create', this.network], { stdio: 'ignore' });
		this.networkCreated = true;

		const config = join(this.dir, 'config.toml');
		writeFileSync(
			config,
			`[http]
base_url = "${url}"
public_addr = "127.0.0.1:${publicPort}"
internal_addr = "127.0.0.1:${internalPort}"
ui_dir = "${join(web, 'build')}"

[identity.local]

[ssh]
public_host = "127.0.0.1"

[engine]
kind = "docker"
network = "${this.network}"

[[catalog.images]]
key = "dev"
label = "Local build"
reference = "${process.env.CORRAL_SANDBOX_IMAGE ?? 'corral-sandbox:dev'}"
`
		);
		const proxyKey = join(this.dir, 'proxy_key');
		execFileSync('ssh-keygen', ['-q', '-t', 'ed25519', '-N', '', '-f', proxyKey]);
		const env = {
			...process.env,
			...keys(binary),
			CORRAL_CONFIG: config,
			CORRAL_PROXY_PUBLIC_KEY: readFileSync(`${proxyKey}.pub`, 'utf8').trim(),
			DATABASE_URL: `sqlite://${join(this.dir, 'corral.db')}`
		};

		const accounts: Record<Role, Account> = {
			admin: { username: 'ada', password: randomBytes(18).toString('base64') },
			member: { username: 'bob', password: randomBytes(18).toString('base64') }
		};
		for (const [role, account] of Object.entries(accounts)) {
			const admin = role === 'admin' ? ['--admin'] : [];
			execFileSync(binary, ['local-account', 'create', account.username, ...admin], {
				env,
				input: `${account.password}\n`,
				stdio: ['pipe', 'ignore', 'inherit']
			});
		}

		const log = openSync(this.log, 'w');
		this.server = spawn(binary, ['serve'], { env, stdio: ['ignore', log, log] });
		await healthy(this.server, url, this.log);
		for (const [role, account] of Object.entries(accounts)) {
			await signIn(url, account, session(role as Role));
		}
		process.env.CORRAL_E2E_URL = url;
		process.env.CORRAL_E2E_ACCOUNTS = JSON.stringify(accounts);
	}

	async stop() {
		const server = this.server;
		if (server && server.exitCode === null) {
			const exited = new Promise((done) => server.once('exit', done));
			server.kill('SIGTERM');
			await exited;
		}
		if (this.networkCreated) {
			removeSandboxes(this.network);
			execFileSync('docker', ['network', 'rm', this.network], { stdio: 'ignore' });
		}
		try {
			mkdirSync(join(web, 'test-results'), { recursive: true });
			copyFileSync(this.log, join(web, 'test-results', 'corral-server.log'));
		} catch {
			// The server never started, so there is no log to keep.
		}
		rmSync(this.dir, { recursive: true, force: true });
	}
}

async function signIn(url: string, account: Account, path: string) {
	const context = await request.newContext({ baseURL: url });
	const response = await context.post('/auth/local/login', { data: account });
	if (!response.ok()) {
		throw new Error(`signing in as ${account.username}: ${await response.text()}`);
	}
	await context.storageState({ path });
	await context.dispose();
}

async function healthy(server: ChildProcess, url: string, log: string) {
	for (let attempt = 0; attempt < 100; attempt++) {
		if (server.exitCode !== null) {
			throw new Error(`corral-server exited:\n${readFileSync(log, 'utf8')}`);
		}
		const ok = await fetch(`${url}/healthz`).then(
			(response) => response.ok,
			() => false
		);
		if (ok) {
			return;
		}
		await sleep(100);
	}
	throw new Error(`corral-server never answered ${url}/healthz`);
}

function serverBinary(): string {
	if (process.env.CORRAL_SERVER_BIN) {
		return process.env.CORRAL_SERVER_BIN;
	}
	execFileSync('cargo', ['build', '--package', 'corral-server'], { cwd: repo, stdio: 'inherit' });
	const metadata = execFileSync('cargo', ['metadata', '--format-version', '1', '--no-deps'], {
		cwd: repo,
		encoding: 'utf8'
	});
	return join(JSON.parse(metadata).target_directory, 'debug', 'corral-server');
}

/** `CORRAL_SESSION_KEY` and `CORRAL_SECRETS_KEY`, fresh from `corral-server keys generate`. */
function keys(binary: string): Record<string, string> {
	const lines = execFileSync(binary, ['keys', 'generate'], { encoding: 'utf8' }).trim().split('\n');
	return Object.fromEntries(lines.map((line) => line.split(/=(.*)/s).slice(0, 2)));
}

function freePort(): Promise<number> {
	return new Promise((resolvePort, reject) => {
		const server = createServer();
		server.once('error', reject);
		server.listen(0, '127.0.0.1', () => {
			const address = server.address();
			server.close(() =>
				typeof address === 'object' && address ? resolvePort(address.port) : reject()
			);
		});
	});
}

/** Removes every sandbox container on the network, with its home volume. */
function removeSandboxes(network: string) {
	const docker = (...args: string[]) =>
		execFileSync('docker', args, { encoding: 'utf8' }).split(/\s+/).filter(Boolean);
	for (const container of docker('ps', '--all', '--quiet', '--filter', `network=${network}`)) {
		const volumes = docker(
			'inspect',
			'--format',
			'{{range .Mounts}}{{if eq .Type "volume"}}{{.Name}} {{end}}{{end}}',
			container
		);
		docker('rm', '--force', container);
		for (const volume of volumes) {
			docker('volume', 'rm', volume);
		}
	}
}
