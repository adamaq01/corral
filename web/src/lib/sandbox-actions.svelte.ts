import { SvelteMap } from 'svelte/reactivity';
import type { Sandbox } from '#lib/api/generated/index.js';
import { sandboxes } from '#lib/api/sandboxes.js';
import { states, type Power, type State } from '#lib/sandbox-status.js';

const calls: Record<Power, (name: string) => Promise<{ ready: boolean } | void>> = {
	wake: sandboxes.wake,
	stop: sandboxes.stop,
	restart: sandboxes.restart
};

const leadsTo: Record<Power, State> = { wake: 'running', stop: 'sleeping', restart: 'running' };

/** How long after a power's request a sandbox not yet in the state it leads to is watched. */
const WATCH_MS = 3 * 60_000;

type Watch = { power: Power; inFlight: boolean };

// By sandbox name, kept across pages so a list opened mid-restart keeps up with it.
const watched = new SvelteMap<string, Watch>();

/**
 * Asks Corral to `power` the sandbox, which every page showing it then watches until it reaches
 * the state the power leads to. False when Corral answered before a wake finished.
 */
export async function runPower(name: string, power: Power): Promise<boolean> {
	const running: Watch = { power, inFlight: true };
	watched.set(name, running);
	let ready: boolean;
	try {
		ready = (await calls[power](name))?.ready ?? true;
	} catch (error) {
		if (watched.get(name) === running) {
			watched.delete(name);
		}
		throw error;
	}
	if (watched.get(name) === running) {
		const done: Watch = { power, inFlight: false };
		watched.set(name, done);
		setTimeout(() => {
			if (watched.get(name) === done) {
				watched.delete(name);
			}
		}, WATCH_MS);
	}
	return ready;
}

/** The power whose request for the sandbox is still in flight. */
export function powerInFlight(name: string): Power | null {
	const watch = watched.get(name);
	return watch?.inFlight ? watch.power : null;
}

/** Whether the sandbox's status is worth fetching again soon. */
export function unsettled(sandbox: Sandbox): boolean {
	if (states[sandbox.state].settling) {
		return true;
	}
	const watch = watched.get(sandbox.name);
	return (
		watch !== undefined &&
		(watch.inFlight || (sandbox.state !== leadsTo[watch.power] && sandbox.state !== 'failed'))
	);
}
