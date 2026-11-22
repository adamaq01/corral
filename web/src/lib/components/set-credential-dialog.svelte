<script lang="ts">
	import { untrack } from 'svelte';
	import { credentials } from '#lib/api/credentials.js';
	import FormDialog from '#lib/components/form-dialog.svelte';
	import * as Field from '#lib/components/ui/field/index.js';
	import { Input } from '#lib/components/ui/input/index.js';

	let {
		open = $bindable(),
		replacing,
		existing,
		onSaved
	}: {
		open: boolean;
		/** The credential whose value this replaces, or `null` to add a new one. */
		replacing: string | null;
		existing: string[];
		onSaved: () => Promise<void>;
	} = $props();

	let name = $state('');
	let value = $state('');

	$effect(() => {
		if (open) {
			untrack(() => {
				name = replacing ?? '';
				value = '';
			});
		}
	});

	const taken = $derived(replacing === null && existing.includes(name));

	async function save() {
		await credentials.set(name, { value });
		await onSaved();
	}
</script>

<FormDialog
	bind:open
	title={replacing ? `Replace ${replacing}` : 'Add a credential'}
	description={replacing
		? "The new value takes the old one's place, which is gone for good."
		: 'Your sandboxes get it as an environment variable. Its value is never shown again.'}
	submit={replacing ? 'Replace value' : 'Save credential'}
	ready={!taken}
	onsubmit={save}
>
	{#if !replacing}
		<Field.Field data-invalid={taken || undefined}>
			<Field.Label for="credential-name">Name</Field.Label>
			<Input
				id="credential-name"
				bind:value={() => name, (typed) => (name = typed.toUpperCase())}
				required
				maxlength={64}
				autocomplete="off"
				autocapitalize="characters"
				spellcheck={false}
				class="font-mono"
				placeholder="OPENAI_API_KEY"
				aria-invalid={taken || undefined}
			/>
			{#if taken}
				<Field.Error>
					{name} exists already. Use Replace on its row to change its value.
				</Field.Error>
			{:else}
				<Field.Description>Capital letters, digits and underscores.</Field.Description>
			{/if}
		</Field.Field>
	{/if}
	<Field.Field>
		<Field.Label for="credential-value">Value</Field.Label>
		<Input
			id="credential-value"
			type="password"
			bind:value
			required
			autocomplete="off"
			spellcheck={false}
		/>
	</Field.Field>
</FormDialog>
