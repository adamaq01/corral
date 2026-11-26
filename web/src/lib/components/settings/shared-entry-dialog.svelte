<script lang="ts">
	import { untrack } from 'svelte';
	import { ApiError } from '#lib/api/client.js';
	import type { EntryKind, Shared, SharedEntry } from '#lib/api/generated/index.js';
	import { changeSharedEntry, createSharedEntry } from '#lib/api/settings.js';
	import EntryFields from '#lib/components/entry-fields.svelte';
	import FormDialog from '#lib/components/form-dialog.svelte';
	import * as Field from '#lib/components/ui/field/index.js';
	import * as RadioGroup from '#lib/components/ui/radio-group/index.js';
	import { EntryForm } from '#lib/entry-form.svelte.js';
	import { collides, noun, settings, where } from '#lib/setup.js';

	let {
		open = $bindable(),
		entry,
		kind,
		others,
		onsaved
	}: {
		open: boolean;
		/** The entry to change, or `null` to add one. */
		entry: SharedEntry | null;
		/** What a new entry is. */
		kind: EntryKind;
		/** The shared setup, whose targets the entry must not collide with. */
		others: SharedEntry[];
		onsaved: (entry: SharedEntry) => Promise<void>;
	} = $props();

	const choices: { shared: Shared; label: string; hint: string }[] = [
		{
			shared: 'required',
			label: 'Required for every sandbox',
			hint: 'Each sandbox gets it at its next start, and members cannot detach, override or replace it.'
		},
		{
			shared: 'offered',
			label: 'Offered, members attach it',
			hint: 'Members attach it to the sandboxes they choose, override it there, or replace it with their own.'
		}
	];

	let form = $state(new EntryForm());
	let shared = $state<Shared>('required');
	/** Corral's refusal of the entry for how it is shared, with what it refused. */
	let refused = $state<{ shared: Shared; kind: EntryKind; target: string; message: string } | null>(
		null
	);
	let fresh = $state(0);

	$effect(() => {
		if (open) {
			untrack(() => {
				form = new EntryForm(entry ? { ...settings(entry), target: where(entry) } : { kind });
				shared = entry?.shared ?? 'required';
				refused = null;
				fresh += 1;
			});
		}
	});

	const taken = $derived(others.find((other) => other.id !== entry?.id && collides(other, form)));
	const refusal = $derived(
		refused?.shared === shared && refused.kind === form.kind && refused.target === form.target
			? refused.message
			: null
	);
	const ready = $derived(
		form.target !== '' &&
			form.problem === null &&
			taken === undefined &&
			(entry !== null || form.value !== null)
	);

	async function save(): Promise<void | false> {
		const change = { ...form.settings, shared };
		let saved: SharedEntry;
		try {
			saved = entry
				? await changeSharedEntry(entry.id, { ...change, value: form.value ?? undefined })
				: await createSharedEntry({ ...change, value: form.value! });
		} catch (failure) {
			if (failure instanceof ApiError && failure.status === 409) {
				refused = { shared, kind: form.kind, target: form.target, message: failure.message };
				return false;
			}
			throw failure;
		}
		await onsaved(saved);
	}
</script>

<FormDialog
	bind:open
	title={entry ? `Edit ${where(entry)}` : `Add a shared ${noun(form.kind)}`}
	description={entry
		? 'Sandboxes get the change at their next start. Corral refuses a change that would keep one of them from starting, and says why.'
		: form.variable
			? 'Each SSH session of the sandboxes it reaches gets it as a variable.'
			: 'Written into the home of the sandboxes it reaches at each start, replacing what is at its path.'}
	submit="Save entry"
	{ready}
	onsubmit={save}
>
	{#key fresh}
		<EntryFields
			{form}
			{entry}
			current={entry?.value}
			taken={taken ? `The shared setup already has ${where(taken)}.` : null}
			newSandboxes={shared === 'offered'
				? {
						label: 'Attach to new sandboxes',
						hint: 'Each sandbox members create from now on gets it, and they may detach it.'
					}
				: null}
		>
			<Field.Set>
				<Field.Legend variant="label">Shared as</Field.Legend>
				<RadioGroup.Root
					bind:value={shared}
					class="grid gap-3"
					aria-describedby={refusal ? 'shared-entry-refusal' : undefined}
				>
					{#each choices as choice (choice.shared)}
						<Field.Field orientation="horizontal">
							<RadioGroup.Item
								value={choice.shared}
								id="shared-entry-{choice.shared}"
								aria-describedby="shared-entry-{choice.shared}-hint"
							/>
							<Field.Content>
								<Field.Label for="shared-entry-{choice.shared}">{choice.label}</Field.Label>
								<Field.Description id="shared-entry-{choice.shared}-hint">
									{choice.hint}
								</Field.Description>
							</Field.Content>
						</Field.Field>
					{/each}
				</RadioGroup.Root>
				{#if refusal}
					<Field.Error id="shared-entry-refusal" class="first-letter:uppercase">
						{refusal}
					</Field.Error>
				{/if}
			</Field.Set>
		</EntryFields>
	{/key}
</FormDialog>
