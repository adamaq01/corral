import { spawn } from 'node:child_process';

/** How an ssh run ended. */
export type Ran = { code: number | null; stdout: string; stderr: string };

/**
 * Runs `remote` over ssh to `destination`, its arguments as `-p <port> user@host` or an `ssh://`
 * URI, with `key` alone and the gate's host key pinned.
 */
export function overSsh(destination: string[], key: string, remote: string): Promise<Ran> {
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
		'PreferredAuthentications=publickey',
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
		const ssh = spawn('ssh', [...options, ...destination, remote], {
			stdio: ['ignore', 'pipe', 'pipe']
		});
		let [stdout, stderr] = ['', ''];
		ssh.stdout.on('data', (data) => (stdout += data));
		ssh.stderr.on('data', (data) => (stderr += data));
		ssh.once('error', reject);
		ssh.once('exit', (code) => resolve({ code, stdout: stdout.trim(), stderr }));
	});
}

/** What `remote` prints over ssh to `destination`, failing unless it exits 0. */
export async function sshOutput(destination: string[], key: string, remote: string) {
	const { code, stdout, stderr } = await overSsh(destination, key, remote);
	if (code !== 0) {
		throw new Error(`ssh ${destination.join(' ')} ${remote} exited ${code}: ${stderr}`);
	}
	return stdout;
}
