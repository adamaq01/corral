import { ApiError, failureMessage } from '#lib/api/client.js';

type RemoteState<T> =
	| { state: 'loading' }
	| { state: 'failed'; error: string; status: number | null }
	| { state: 'ready'; value: T };

/** Corral or a proxy in front of it failed for now, and a later call may well succeed. */
function transient(status: number | null): boolean {
	return status === 0 || (status !== null && status >= 500);
}

/** A value a page fetches from the API, starting at once, with its loading and failed states. */
export class Remote<T> {
	current = $state<RemoteState<T>>({ state: 'loading' });
	/** Why the latest reload failed while an older value stays shown. */
	refreshFailure = $state<string | null>(null);
	/** When the value shown arrived. */
	fetchedAt = $state<Date | null>(null);
	readonly #fetch: () => Promise<T>;
	#latest = 0;

	constructor(fetch: () => Promise<T>) {
		this.#fetch = fetch;
		void this.reload();
	}

	get value(): T | undefined {
		return this.current.state === 'ready' ? this.current.value : undefined;
	}

	/**
	 * Fetches the value again, keeping any value shown until the new one arrives, and through a
	 * failure that may pass, such as Corral being unreachable or a proxy's 502, so the next reload
	 * can catch up. Only the latest call lands.
	 */
	async reload() {
		const call = ++this.#latest;
		if (this.current.state === 'failed') {
			this.current = { state: 'loading' };
		}
		try {
			const value = await this.#fetch();
			if (call === this.#latest) {
				this.current = { state: 'ready', value };
				this.refreshFailure = null;
				this.fetchedAt = new Date();
			}
		} catch (error) {
			if (call !== this.#latest) {
				return;
			}
			const status = error instanceof ApiError ? error.status : null;
			if (this.current.state === 'ready' && transient(status)) {
				this.refreshFailure = failureMessage(error);
				return;
			}
			this.current = { state: 'failed', error: failureMessage(error), status };
			this.refreshFailure = null;
		}
	}

	/** Changes the value shown after an action the API confirmed. */
	update(change: (value: T) => T) {
		if (this.current.state === 'ready') {
			this.current = { state: 'ready', value: change(this.current.value) };
		}
	}
}

const BUSY_MS = 2000;
const IDLE_MS = 30_000;
/** A tab shown again or focused within this long of the last reload skips another. */
const FRESH_MS = 1000;

/**
 * Keeps `remote` fresh while the page shows it: every two seconds while `busy` holds, such as
 * while a sandbox starts, else every 30 seconds. It pauses in a hidden tab and reloads at once
 * when the tab is shown or focused again.
 */
export function keepFresh(remote: Remote<unknown>, busy: () => boolean = () => false) {
	$effect(() => {
		const period = busy() ? BUSY_MS : IDLE_MS;
		let timer: ReturnType<typeof setTimeout> | undefined;
		let stopped = false;
		let last = Date.now();

		const schedule = () => {
			clearTimeout(timer);
			if (!stopped && !document.hidden) {
				timer = setTimeout(tick, period);
			}
		};
		const tick = async () => {
			last = Date.now();
			await remote.reload();
			schedule();
		};
		const resume = () => {
			if (document.hidden) {
				clearTimeout(timer);
			} else if (Date.now() - last > FRESH_MS) {
				void tick();
			} else {
				schedule();
			}
		};

		schedule();
		document.addEventListener('visibilitychange', resume);
		window.addEventListener('focus', resume);
		return () => {
			stopped = true;
			clearTimeout(timer);
			document.removeEventListener('visibilitychange', resume);
			window.removeEventListener('focus', resume);
		};
	});
}
