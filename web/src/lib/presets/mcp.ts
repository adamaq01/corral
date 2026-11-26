import type { SetupEntry } from '../api/generated/index.js';
import { isObject, own, ownJson, put, type Change } from './change.js';

const SERVERS = { kind: 'file', target: '.config/corral-agents/mcp.json' } as const;

export type McpServer = { name: string } & (
	| { runs: 'command'; command: string; args: string[]; env: Record<string, string> }
	| { runs: 'url'; url: string; headers: Record<string, string> }
);

/** The servers, or only that the file is secret, as a secret file never shows. */
export type McpState = { secret: false; servers: McpServer[] } | { secret: true };

export type McpForm = { secret: boolean; servers: McpServer[] };

export function readMcp(entries: SetupEntry[]): McpState {
	return own(entries, SERVERS)?.secret
		? { secret: true }
		: { secret: false, servers: servers(ownJson(entries, SERVERS)) };
}

export function writeMcp(current: SetupEntry[], form: McpForm): Change[] {
	const file = Object.fromEntries(
		form.servers.map((server) => [
			server.name,
			server.runs === 'command'
				? { command: server.command, args: server.args, env: server.env }
				: { type: 'http', url: server.url, headers: server.headers }
		])
	);
	const text = form.servers.length > 0 ? `${JSON.stringify(file, null, 2)}\n` : null;
	return put(current, { ...SERVERS, secret: form.secret }, text);
}

/** `N servers: a, b, c`, `Secret`, or `null` when there are none. */
export function mcpSummary(state: McpState): string | null {
	if (state.secret) {
		return 'Secret';
	}
	const count = state.servers.length;
	if (count === 0) {
		return null;
	}
	const names = state.servers.map((server) => server.name).join(', ');
	return `${count} ${count === 1 ? 'server' : 'servers'}: ${names}`;
}

/** The servers of a file the member may also have edited by hand, skipping what is not one. */
function servers(file: Record<string, unknown>): McpServer[] {
	return Object.entries(file).flatMap(([name, server]): McpServer[] => {
		if (!isObject(server)) {
			return [];
		}
		if (typeof server.url === 'string') {
			return [{ name, runs: 'url', url: server.url, headers: strings(server.headers) }];
		}
		return [
			{
				name,
				runs: 'command',
				command: typeof server.command === 'string' ? server.command : '',
				args: Array.isArray(server.args) ? server.args.map(String) : [],
				env: strings(server.env)
			}
		];
	});
}

function strings(value: unknown): Record<string, string> {
	return isObject(value)
		? Object.fromEntries(Object.entries(value).map(([key, held]) => [key, String(held)]))
		: {};
}
