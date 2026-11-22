<script lang="ts">
	import { untrack } from 'svelte';
	import type { Catalog, Sandbox } from '#lib/api/generated/index.js';
	import { sandboxes } from '#lib/api/sandboxes.js';
	import FormDialog from '#lib/components/form-dialog.svelte';
	import * as Field from '#lib/components/ui/field/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import * as NativeSelect from '#lib/components/ui/native-select/index.js';

	let {
		open = $bindable(),
		catalog,
		count,
		onCreated
	}: {
		open: boolean;
		catalog: Catalog;
		/** How many sandboxes the user has now. */
		count: number;
		onCreated: (sandbox: Sandbox) => Promise<void>;
	} = $props();

	let name = $state('');
	let image = $state('');
	let size = $state('');

	$effect(() => {
		if (open) {
			untrack(() => {
				name = count === 0 ? 'main' : '';
				image = catalog.defaultImage;
				size = catalog.defaultSize;
			});
		}
	});

	const full = $derived(count >= catalog.maxSandboxes);

	async function create() {
		await onCreated(await sandboxes.create({ name, image, size }));
	}
</script>

<FormDialog
	bind:open
	title="New sandbox"
	description={full
		? `You have ${count} of ${catalog.maxSandboxes} sandboxes, the most you can have. Delete one to make room.`
		: `A persistent machine with its own home directory. You have ${count} of ${catalog.maxSandboxes} sandboxes.`}
	submit="Create sandbox"
	ready={!full}
	onsubmit={create}
>
	<Field.Field>
		<Field.Label for="sandbox-name">Name</Field.Label>
		<Input
			id="sandbox-name"
			bind:value={name}
			required
			maxlength={32}
			autocomplete="off"
			autocapitalize="none"
			spellcheck={false}
			disabled={full}
		/>
		<Field.Description>
			Lowercase letters, digits and dashes. It becomes part of the SSH user name.
		</Field.Description>
	</Field.Field>
	<Field.Field>
		<Field.Label for="sandbox-image">Image</Field.Label>
		<NativeSelect.Root id="sandbox-image" bind:value={image} class="w-full" disabled={full}>
			{#each catalog.images as choice (choice.key)}
				<NativeSelect.Option value={choice.key}>{choice.label}</NativeSelect.Option>
			{/each}
		</NativeSelect.Root>
	</Field.Field>
	<Field.Field>
		<Field.Label for="sandbox-size">Size</Field.Label>
		<NativeSelect.Root id="sandbox-size" bind:value={size} class="w-full" disabled={full}>
			{#each catalog.sizes as choice (choice.key)}
				<NativeSelect.Option value={choice.key}>{choice.label}</NativeSelect.Option>
			{/each}
		</NativeSelect.Root>
	</Field.Field>
</FormDialog>
