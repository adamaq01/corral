import type { OpksshLogin, Sandbox } from '#lib/api/generated/index.js';

export type ConnectSnippet = { label: string; text: string };

/** The `~/.ssh/config` alias for a sandbox, so `ssh <alias>` reaches it. */
export function hostAlias(sandbox: Sandbox): string {
	return `corral-${sandbox.name}`;
}

/** The command that reaches the sandbox over SSH. */
export function sshCommand(sandbox: Sandbox): string {
	const { username, host, port } = sandbox.ssh;
	return `ssh -p ${port} ${username}@${host}`;
}

/**
 * How to reach a sandbox from an SSH client, from Herdr, and from an SSH config file, and for the
 * default sandbox, by its owner's `handle` alone.
 */
export function connectSnippets(sandbox: Sandbox, handle: string): ConnectSnippet[] {
	const { username, host, port } = sandbox.ssh;
	return [
		{ label: 'SSH', text: sshCommand(sandbox) },
		...(sandbox.isDefault
			? [{ label: 'SSH to your default sandbox', text: `ssh -p ${port} ${handle}@${host}` }]
			: []),
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

/** The opkssh provider alias, and the key `opkssh login` writes for Corral. */
export const OPKSSH_ALIAS = 'corral';
const OPKSSH_KEY = '~/.ssh/corral-opkssh';
/** How long ssh waits for a login before it connects without one. */
export const OPKSSH_LOGIN_TIMEOUT_SECS = 30;

/**
 * The laptop's opkssh client config, and an SSH config block that renews the certificate before
 * ssh connects to `host` when it is missing or within an hour of expiring. opkssh's certificates
 * say nothing of their expiry, so the hook goes by the age of the file `opkssh login` wrote.
 */
export function opksshSnippets(login: OpksshLogin, host: string): ConnectSnippet[] {
	const renewAfterMinutes = (login.lifetimeHours - 1) * 60;
	const renew = [
		`find ${OPKSSH_KEY}-cert.pub -mmin -${renewAfterMinutes} 2>/dev/null | grep -q .`,
		`|| { opkssh login ${OPKSSH_ALIAS} -i ${OPKSSH_KEY} & p=$!;`,
		`(sleep ${OPKSSH_LOGIN_TIMEOUT_SECS}; kill $p) & wait $p; }; true`
	].join(' ');
	return [
		{
			label: 'opkssh config (~/.opk/config.yml)',
			text: [
				'providers:',
				`  - alias: ${OPKSSH_ALIAS}`,
				`    issuer: ${login.issuer}`,
				`    client_id: ${login.clientId}`,
				'    scopes: openid profile email',
				'    prompt: ""'
			].join('\n')
		},
		{
			label: 'SSH config (end of ~/.ssh/config)',
			text: [
				`Match host ${host} exec "sh -c '${renew}' >/dev/null 2>&1"`,
				`  IdentityFile ${OPKSSH_KEY}`
			].join('\n')
		}
	];
}
