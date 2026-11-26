import { toast } from 'svelte-sonner';
import { failureMessage } from '#lib/api/client.js';
import { Remote } from '#lib/remote.svelte.js';

/** A settings page's view as Corral has it, and as the admin changes it until one save sends it. */
export class SettingsForm<V> {
	readonly loaded: Remote<V>;
	/** The view as the admin changes it, `null` until it loads. */
	edited = $state<V | null>(null);
	saving = $state(false);
	/** Why the latest save failed, in Corral's words. */
	failure = $state<string | null>(null);
	readonly #send: (view: V) => Promise<void>;
	readonly #saved: string;

	/** `saved` is the toast a save that worked shows. */
	constructor(load: () => Promise<V>, send: (view: V) => Promise<void>, saved: string) {
		this.#send = send;
		this.#saved = saved;
		this.loaded = new Remote(async () => {
			const view = await load();
			this.edited = structuredClone(view);
			return view;
		});
	}

	get dirty(): boolean {
		return (
			this.edited !== null && JSON.stringify(this.edited) !== JSON.stringify(this.loaded.value)
		);
	}

	async save() {
		if (this.edited === null) {
			return;
		}
		this.saving = true;
		this.failure = null;
		try {
			await this.#send(this.edited);
			toast.success(this.#saved);
			await this.loaded.reload();
		} catch (error) {
			this.failure = failureMessage(error);
		} finally {
			this.saving = false;
		}
	}

	reset() {
		this.edited = $state.snapshot(this.loaded.value) as V;
		this.failure = null;
	}
}
