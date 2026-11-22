import { ApiError, failureMessage } from '#lib/api/client.js';

type RemoteState<T> =
	| { state: 'loading' }
	| { state: 'failed'; error: string; status: number | null }
	| { state: 'ready'; value: T };

/** A value a page fetches from the API, starting at once, with its loading and failed states. */
export class Remote<T> {
	current = $state<RemoteState<T>>({ state: 'loading' });
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
	 * moment when Corral cannot be reached, so the next reload can catch up. Only the latest call
	 * lands.
	 */
	async reload() {
		const call = ++this.#latest;
		if (this.current.state === 'failed') {
			this.current = { state: 'loading' };
		}
		let next: RemoteState<T>;
		try {
			next = { state: 'ready', value: await this.#fetch() };
		} catch (error) {
			const status = error instanceof ApiError ? error.status : null;
			if (status === 0 && this.current.state === 'ready') {
				return;
			}
			next = { state: 'failed', error: failureMessage(error), status };
		}
		if (call === this.#latest) {
			this.current = next;
		}
	}

	/** Changes the value shown after an action the API confirmed. */
	update(change: (value: T) => T) {
		if (this.current.state === 'ready') {
			this.current = { state: 'ready', value: change(this.current.value) };
		}
	}
}

/** Fetches `remote` again every two seconds for as long as `active` stays true. */
export function pollWhile(active: () => boolean, remote: Remote<unknown>) {
	$effect(() => {
		if (!active()) {
			return;
		}
		const timer = setInterval(() => void remote.reload(), 2000);
		return () => clearInterval(timer);
	});
}
