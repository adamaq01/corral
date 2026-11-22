import { failureMessage } from '#lib/api/client.js';

type RemoteState<T> =
	{ state: 'loading' } | { state: 'failed'; error: string } | { state: 'ready'; value: T };

/** A value a page fetches from the API, starting at once, with its loading and failed states. */
export class Remote<T> {
	current = $state<RemoteState<T>>({ state: 'loading' });
	readonly #fetch: () => Promise<T>;

	constructor(fetch: () => Promise<T>) {
		this.#fetch = fetch;
		void this.reload();
	}

	/** Fetches the value again, keeping any value shown until the new one arrives. */
	async reload() {
		if (this.current.state === 'failed') {
			this.current = { state: 'loading' };
		}
		try {
			this.current = { state: 'ready', value: await this.#fetch() };
		} catch (error) {
			this.current = { state: 'failed', error: failureMessage(error) };
		}
	}

	/** Changes the value shown after an action the API confirmed. */
	update(change: (value: T) => T) {
		if (this.current.state === 'ready') {
			this.current = { state: 'ready', value: change(this.current.value) };
		}
	}
}
