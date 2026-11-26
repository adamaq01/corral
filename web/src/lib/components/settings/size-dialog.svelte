<script lang="ts">
	import { untrack } from 'svelte';
	import FormDialog from '#lib/components/form-dialog.svelte';
	import * as Field from '#lib/components/ui/field/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { BYTES_PATTERN, CPU_PATTERN, KEY_PATTERN, type SizeEntry } from '#lib/settings.js';

	let {
		open = $bindable(),
		size,
		refuse,
		onsave
	}: {
		open: boolean;
		/** The size to change, or `null` to add one. */
		size: SizeEntry | null;
		/** Why a key cannot name a new size, if it cannot. */
		refuse: (key: string) => string | null;
		onsave: (size: SizeEntry) => void;
	} = $props();

	const added: SizeEntry = {
		key: '',
		label: '',
		cpuRequest: '',
		cpuLimit: '',
		memoryRequest: '',
		memoryLimit: '',
		homeSize: '',
		enabled: true
	};
	let fields = $state({ ...added });

	$effect(() => {
		if (open) {
			untrack(() => {
				fields = { ...(size ?? added) };
			});
		}
	});

	const refused = $derived(size || fields.key === '' ? null : refuse(fields.key));

	const BYTES_HINT = 'As in Kubernetes: 4Gi, 512Mi or 1G.';
	const quantities = [
		{
			hint: { id: 'size-cpu-hint', text: 'As in Kubernetes: 2, 1.5 or 500m.' },
			pattern: CPU_PATTERN,
			inputs: [
				{ id: 'size-cpu-request', label: 'CPU request', field: 'cpuRequest' },
				{ id: 'size-cpu-limit', label: 'CPU limit', field: 'cpuLimit' }
			]
		},
		{
			hint: { id: 'size-memory-hint', text: BYTES_HINT },
			pattern: BYTES_PATTERN,
			inputs: [
				{ id: 'size-memory-request', label: 'Memory request', field: 'memoryRequest' },
				{ id: 'size-memory-limit', label: 'Memory limit', field: 'memoryLimit' }
			]
		},
		{
			hint: { id: 'size-home-hint', text: BYTES_HINT },
			pattern: BYTES_PATTERN,
			inputs: [{ id: 'size-home', label: 'Home size', field: 'homeSize' }]
		}
	] as const;

	async function save() {
		const { key, label, cpuRequest, cpuLimit, memoryRequest, memoryLimit, homeSize, enabled } =
			fields;
		onsave({
			key: size?.key ?? key,
			label: label.trim(),
			cpuRequest: cpuRequest.trim(),
			cpuLimit: cpuLimit.trim(),
			memoryRequest: memoryRequest.trim(),
			memoryLimit: memoryLimit.trim(),
			homeSize: homeSize.trim(),
			enabled
		});
	}
</script>

<FormDialog
	bind:open
	title={size ? `Edit ${size.key}` : 'Add size'}
	description={size
		? 'The catalog changes once you save it.'
		: 'A sandbox is sure of its requests and may use up to its limits. The catalog offers the size once you save it.'}
	submit={size ? 'Change size' : 'Add size'}
	ready={refused === null}
	onsubmit={save}
>
	{#if !size}
		<Field.Field data-invalid={refused ? true : undefined}>
			<Field.Label for="size-key">Key</Field.Label>
			<Input
				id="size-key"
				autocomplete="off"
				autocapitalize="none"
				spellcheck={false}
				pattern={KEY_PATTERN}
				required
				aria-invalid={refused ? true : undefined}
				aria-describedby="size-key-hint"
				bind:value={fields.key}
			/>
			{#if refused}
				<Field.Error id="size-key-hint">{refused}</Field.Error>
			{:else}
				<Field.Description id="size-key-hint">
					Up to 32 lowercase letters, digits and dashes. The sandboxes made from the size name it,
					so it stays.
				</Field.Description>
			{/if}
		</Field.Field>
	{/if}
	<Field.Field>
		<Field.Label for="size-label">Label</Field.Label>
		<Input id="size-label" required aria-describedby="size-label-hint" bind:value={fields.label} />
		<Field.Description id="size-label-hint">
			What members see when they pick it, such as "Small: 2 CPUs, 4 GiB".
		</Field.Description>
	</Field.Field>
	{#each quantities as { hint, pattern, inputs } (hint.id)}
		<div class="grid gap-2">
			<div class="grid gap-4 sm:grid-cols-2">
				{#each inputs as { id, label, field } (id)}
					<Field.Field>
						<Field.Label for={id}>{label}</Field.Label>
						<Input
							{id}
							autocomplete="off"
							autocapitalize="none"
							spellcheck={false}
							{pattern}
							required
							aria-describedby={hint.id}
							bind:value={fields[field]}
						/>
					</Field.Field>
				{/each}
			</div>
			<Field.Description id={hint.id}>{hint.text}</Field.Description>
		</div>
	{/each}
</FormDialog>
