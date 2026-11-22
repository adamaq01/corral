import type { Sandbox } from '#lib/api/generated/index.js';

export type ConnectSnippet = { label: string; text: string };

/** The `~/.ssh/config` alias for a sandbox, so `ssh <alias>` reaches it. */
export function hostAlias(sandbox: Sandbox): string {
	return `corral-${sandbox.name}`;
}

/** How to reach a sandbox from an SSH client, from Herdr, and from an SSH config file. */
export function connectSnippets(sandbox: Sandbox): ConnectSnippet[] {
	const { username, host, port } = sandbox.ssh;
	return [
		{ label: 'SSH', text: `ssh -p ${port} ${username}@${host}` },
		{ label: 'Herdr', text: `hc machine add ${username}@${host}:${port} --name ${sandbox.name}` },
		{
			label: 'SSH config',
			text: [
				`Host ${hostAlias(sandbox)}`,
				`  HostName ${host}`,
				`  Port ${port}`,
				`  User ${username}`
			].join('\n')
		}
	];
}
