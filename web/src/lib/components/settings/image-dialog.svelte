<script lang="ts">
	import { untrack } from 'svelte';
	import FormDialog from '#lib/components/form-dialog.svelte';
	import * as Field from '#lib/components/ui/field/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';
	import { KEY_PATTERN, type ImageEntry } from '#lib/settings.js';

	let {
		open = $bindable(),
		image,
		refuse,
		onsave
	}: {
		open: boolean;
		/** The image to change, or `null` to add one. */
		image: ImageEntry | null;
		/** Why a key cannot name a new image, if it cannot. */
		refuse: (key: string) => string | null;
		onsave: (image: ImageEntry) => void;
	} = $props();

	const added: ImageEntry = { key: '', label: '', reference: '', description: '', enabled: true };
	let fields = $state({ ...added });

	$effect(() => {
		if (open) {
			untrack(() => {
				fields = { ...(image ?? added) };
			});
		}
	});

	const refused = $derived(image || fields.key === '' ? null : refuse(fields.key));

	async function save() {
		const { key, label, reference, description, enabled } = fields;
		onsave({
			key: image?.key ?? key,
			label: label.trim(),
			reference: reference.trim(),
			description: description.trim(),
			enabled
		});
	}
</script>

<FormDialog
	bind:open
	title={image ? `Edit ${image.key}` : 'Add image'}
	description={image
		? 'The catalog changes once you save it.'
		: 'An image members can start a sandbox from, offered once you save the catalog.'}
	submit={image ? 'Change image' : 'Add image'}
	ready={refused === null}
	onsubmit={save}
>
	{#if !image}
		<Field.Field data-invalid={refused ? true : undefined}>
			<Field.Label for="image-key">Key</Field.Label>
			<Input
				id="image-key"
				autocomplete="off"
				autocapitalize="none"
				spellcheck={false}
				pattern={KEY_PATTERN}
				required
				aria-invalid={refused ? true : undefined}
				aria-describedby="image-key-hint"
				bind:value={fields.key}
			/>
			{#if refused}
				<Field.Error id="image-key-hint">{refused}</Field.Error>
			{:else}
				<Field.Description id="image-key-hint">
					Up to 32 lowercase letters, digits and dashes. The sandboxes made from the image name it,
					so it stays.
				</Field.Description>
			{/if}
		</Field.Field>
	{/if}
	<Field.Field>
		<Field.Label for="image-label">Label</Field.Label>
		<Input
			id="image-label"
			required
			aria-describedby="image-label-hint"
			bind:value={fields.label}
		/>
		<Field.Description id="image-label-hint">What members see when they pick it.</Field.Description>
	</Field.Field>
	<Field.Field>
		<Field.Label for="image-reference">Reference</Field.Label>
		<Input
			id="image-reference"
			autocomplete="off"
			autocapitalize="none"
			spellcheck={false}
			required
			placeholder="ghcr.io/acme/sandbox:1"
			bind:value={fields.reference}
		/>
	</Field.Field>
	<Field.Field>
		<Field.Label for="image-description">Description</Field.Label>
		<Textarea
			id="image-description"
			aria-describedby="image-description-hint"
			bind:value={fields.description}
		/>
		<Field.Description id="image-description-hint">
			Optional. A sentence on what it holds.
		</Field.Description>
	</Field.Field>
</FormDialog>
