import type { EntryChange, EntryKind, ValueInput } from '#lib/api/generated/index.js';
import { homePath, nameProblem, pathProblem } from '#lib/setup.js';

/** An entry's settings, its file path as typed or as Corral keeps it. */
export type EntrySettings = Omit<EntryChange, 'value'>;

/** What the entry dialogs hold: an entry's settings as typed, and its value. */
export class EntryForm {
	kind = $state<EntryKind>('variable');
	/** The name, or the path, which may start with `~/`. */
	typed = $state('');
	secret = $state(false);
	template = $state(false);
	executable = $state(false);
	applyToNew = $state(true);
	/** The value to send, `null` until the value field holds a valid one. */
	value = $state<ValueInput | null>(null);

	readonly variable = $derived(this.kind === 'variable');
	/** The name, or the path relative to the home, as the API takes it. */
	readonly target = $derived(this.variable ? this.typed : homePath(this.typed));
	readonly problem = $derived(this.variable ? nameProblem(this.typed) : pathProblem(this.typed));

	constructor(start: Partial<EntrySettings> = {}) {
		this.kind = start.kind ?? this.kind;
		this.typed = start.target ?? this.typed;
		this.secret = start.secret ?? this.secret;
		this.template = start.template ?? this.template;
		this.executable = start.executable ?? this.executable;
		this.applyToNew = start.applyToNew ?? this.applyToNew;
	}

	/** The settings as the API takes them. */
	get settings(): EntrySettings {
		return {
			kind: this.kind,
			target: this.target,
			secret: this.secret,
			template: !this.variable && this.template,
			executable: !this.variable && this.executable,
			applyToNew: this.applyToNew
		};
	}
}
