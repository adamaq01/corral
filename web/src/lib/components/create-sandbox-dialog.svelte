<script lang="ts">
	import { untrack } from 'svelte';
	import type { Catalog, Sandbox } from '#lib/api/generated/index.js';
	import { sandboxes } from '#lib/api/sandboxes.js';
	import { bytesLabel } from '#lib/catalog.js';
	import FormDialog from '#lib/components/form-dialog.svelte';
	import * as Field from '#lib/components/ui/field/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import * as NativeSelect from '#lib/components/ui/native-select/index.js';

	let {
		open = $bindable(),
		catalog,
		handle,
		count,
		onCreated
	}: {
		open: boolean;
		catalog: Catalog;
		/** The signed-in user's handle, the second half of each sandbox's SSH username. */
		handle: string;
		/** How many sandboxes the user has now. */
		count: number;
		onCreated: (sandbox: Sandbox) => Promise<void>;
	} = $props();

	// Never a catalog key, which holds only lowercase letters, digits and dashes.
	const NAMED = '+named';

	let name = $state('');
	let image = $state('');
	let reference = $state('');
	let size = $state('');

	$effect(() => {
		if (open) {
			untrack(() => {
				name = count === 0 ? 'main' : '';
				image = catalog.defaultImage;
				reference = '';
				size = catalog.defaultSize;
			});
		}
	});

	const full = $derived(count >= catalog.maxSandboxes);

	const named = $derived(catalog.namedImages && image === NAMED ? catalog.namedImages : undefined);

	const NAME_PATTERN = /^[a-z0-9-]{1,32}$/;
	const preview = $derived(
		`ssh -p ${catalog.ssh.port} ${NAME_PATTERN.test(name) ? name : '<name>'}.${handle}@${catalog.ssh.host}`
	);

	async function create() {
		const choice = named ? { imageReference: reference.trim() } : { image };
		await onCreated(await sandboxes.create({ name, ...choice, size }));
	}
</script>

<FormDialog
	bind:open
	title="New sandbox"
	description={full
		? `You have ${count} of ${catalog.maxSandboxes} sandboxes, the most you can have. Delete one to make room.`
		: `A persistent machine with its own home directory. You have ${count} of ${catalog.maxSandboxes} sandboxes.`}
	submit="Create sandbox"
	pendingLabel={named ? 'Checking the image…' : 'Creating…'}
	ready={!full}
	onsubmit={create}
>
	<Field.Field>
		<Field.Label for="sandbox-name">Name</Field.Label>
		<Input
			id="sandbox-name"
			aria-describedby="sandbox-name-hint"
			bind:value={name}
			required
			maxlength={32}
			pattern={NAME_PATTERN.source.slice(1, -1)}
			autocomplete="off"
			autocapitalize="none"
			spellcheck={false}
			disabled={full}
		/>
		<Field.Description id="sandbox-name-hint">
			Up to 32 lowercase letters, digits and dashes. You then connect with
			<code class="font-mono text-xs wrap-anywhere">{preview}</code>.
		</Field.Description>
	</Field.Field>
	<Field.Field>
		<Field.Label for="sandbox-image">Image</Field.Label>
		<NativeSelect.Root id="sandbox-image" bind:value={image} class="w-full" disabled={full}>
			{#each catalog.images as choice (choice.key)}
				<NativeSelect.Option value={choice.key}>{choice.label}</NativeSelect.Option>
			{/each}
			{#if catalog.namedImages}
				<NativeSelect.Option value={NAMED}>Another image</NativeSelect.Option>
			{/if}
		</NativeSelect.Root>
	</Field.Field>
	{#if named}
		<Field.Field>
			<Field.Label for="sandbox-image-reference">Image reference</Field.Label>
			<Input
				id="sandbox-image-reference"
				aria-describedby="sandbox-image-reference-hint"
				bind:value={reference}
				required
				placeholder="debian:13"
				autocomplete="off"
				autocapitalize="none"
				spellcheck={false}
				disabled={full}
			/>
			<Field.Description id="sandbox-image-reference-hint">
				Any Linux image from {named.registries.join(', ')}, at most {bytesLabel(named.maxBytes)}
				to download. Corral brings the sandbox user, SSH and Herdr to it.
			</Field.Description>
		</Field.Field>
	{/if}
	<Field.Field>
		<Field.Label for="sandbox-size">Size</Field.Label>
		<NativeSelect.Root id="sandbox-size" bind:value={size} class="w-full" disabled={full}>
			{#each catalog.sizes as choice (choice.key)}
				<NativeSelect.Option value={choice.key}>{choice.label}</NativeSelect.Option>
			{/each}
		</NativeSelect.Root>
	</Field.Field>
</FormDialog>
