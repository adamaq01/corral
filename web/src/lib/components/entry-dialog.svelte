<script lang="ts">
	import { untrack } from 'svelte';
	import { setup } from '#lib/api/setup.js';
	import type { SetupEntry } from '#lib/api/generated/index.js';
	import EntryFields from '#lib/components/entry-fields.svelte';
	import FormDialog from '#lib/components/form-dialog.svelte';
	import { EntryForm, type EntrySettings } from '#lib/entry-form.svelte.js';
	import { noun, settings, where } from '#lib/setup.js';

	let {
		open = $bindable(),
		entry = null,
		start,
		title,
		onsaved
	}: {
		open: boolean;
		/** The entry to change; without one, the dialog makes a new entry. */
		entry?: SetupEntry | null;
		/** What a new entry starts as. */
		start?: Partial<EntrySettings>;
		/** The dialog's title in place of the one it gives itself. */
		title?: string;
		onsaved: (entry: SetupEntry) => Promise<void>;
	} = $props();

	let form = $state(new EntryForm());
	let fresh = $state(0);

	$effect(() => {
		if (open) {
			untrack(() => {
				form = new EntryForm(entry ? { ...settings(entry), target: where(entry) } : start);
				fresh += 1;
			});
		}
	});

	const ready = $derived(
		form.target !== '' && form.problem === null && (entry !== null || form.value !== null)
	);

	async function save() {
		const saved = entry
			? await setup.change(entry.id, form.settings)
			: await setup.create({ ...form.settings, value: form.value! });
		await onsaved(saved);
	}
</script>

<FormDialog
	bind:open
	title={title ??
		(entry ? `Edit ${where(entry)}` : form.variable ? 'Add a variable' : 'Add a file')}
	description={entry
		? 'Its sandboxes and their overrides stay. Corral refuses a change that would keep one of them from starting, and says why.'
		: form.variable
			? 'Each SSH session of the sandboxes it is attached to gets it as a variable.'
			: 'Written into the home of the sandboxes it is attached to at each start, replacing what is at its path.'}
	submit={entry ? 'Save changes' : `Save ${noun(form.kind)}`}
	{ready}
	onsubmit={save}
>
	{#key fresh}
		<EntryFields
			{form}
			{entry}
			newSandboxes={{
				label: 'Apply to new sandboxes',
				hint: 'Each sandbox you create from now on gets it.'
			}}
		/>
	{/key}
</FormDialog>
