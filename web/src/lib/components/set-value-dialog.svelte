<script lang="ts">
	import { untrack } from 'svelte';
	import type { EntryKind, StoredValue, ValueInput } from '#lib/api/generated/index.js';
	import FormDialog from '#lib/components/form-dialog.svelte';
	import ValueField from '#lib/components/value-field.svelte';

	let {
		open = $bindable(),
		title,
		description,
		submit,
		kind,
		secret,
		name = '',
		current = null,
		onsave
	}: {
		open: boolean;
		title: string;
		description: string;
		submit: string;
		kind: EntryKind;
		secret: boolean;
		/** The variable's name, which counts toward its size. */
		name?: string;
		/** The value the new one replaces, if any. */
		current?: StoredValue | null;
		onsave: (value: ValueInput) => Promise<void>;
	} = $props();

	let value = $state<ValueInput | null>(null);
	let fresh = $state(0);

	$effect(() => {
		if (open) {
			untrack(() => (fresh += 1));
		}
	});

	async function save() {
		if (value) {
			await onsave(value);
		}
	}
</script>

<FormDialog bind:open {title} {description} {submit} ready={value !== null} onsubmit={save}>
	{#key fresh}
		<ValueField id="set-value" {kind} {secret} {name} {current} bind:value />
	{/key}
</FormDialog>
