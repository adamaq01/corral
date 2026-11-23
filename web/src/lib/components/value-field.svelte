<script lang="ts">
	import EyeIcon from '@lucide/svelte/icons/eye';
	import EyeOffIcon from '@lucide/svelte/icons/eye-off';
	import { untrack } from 'svelte';
	import type { EntryKind, ValueInput } from '#lib/api/generated/index.js';
	import { FILE_LIMIT } from '#lib/api/generated/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Field from '#lib/components/ui/field/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';
	import { fileValue, size } from '#lib/environment.js';

	let {
		id,
		kind,
		secret,
		value = $bindable(null),
		text = '',
		label = 'Value'
	}: {
		id: string;
		kind: EntryKind;
		secret: boolean;
		/** What the field holds, ready to send, or `null` until it holds something valid. */
		value: ValueInput | null;
		/** The text it starts with. */
		text?: string;
		label?: string;
	} = $props();

	let typed = $state(untrack(() => text));
	let shown = $state(false);
	let upload = $state<{ name: string; size: number; value: ValueInput } | null>(null);
	let tooBig = $state<string | null>(null);
	let chosen = $state('');

	$effect(() => {
		if (upload) {
			value = upload.value;
		} else if (kind === 'variable') {
			value = typed === '' ? null : { text: typed };
		} else {
			value = { text: typed };
		}
	});

	async function picked(event: Event) {
		const input = event.currentTarget as HTMLInputElement;
		const file = input.files?.[0];
		tooBig = null;
		upload = null;
		if (!file) {
			return;
		}
		if (file.size > FILE_LIMIT) {
			tooBig = `${file.name} is ${size(file.size)}, more than the ${size(FILE_LIMIT)} a file may hold.`;
			chosen = '';
			return;
		}
		upload = { name: file.name, size: file.size, value: await fileValue(file) };
	}

	function clearUpload() {
		upload = null;
		chosen = '';
	}
</script>

{#if kind === 'variable'}
	<Field.Field>
		<Field.Label for={id}>{label}</Field.Label>
		<div class="flex gap-2">
			<Input
				{id}
				type={secret && !shown ? 'password' : 'text'}
				bind:value={typed}
				required
				autocomplete="off"
				autocapitalize="off"
				spellcheck={false}
				class="font-mono"
			/>
			{#if secret}
				<Button
					type="button"
					variant="outline"
					size="icon"
					aria-label="Show the value"
					aria-pressed={shown}
					aria-controls={id}
					onclick={() => (shown = !shown)}
				>
					{#if shown}
						<EyeOffIcon />
					{:else}
						<EyeIcon />
					{/if}
				</Button>
			{/if}
		</div>
	</Field.Field>
{:else}
	<Field.Field>
		<Field.Label for={id}>{label}</Field.Label>
		{#if upload}
			<div class="flex flex-wrap items-center justify-between gap-2 rounded-md border px-3 py-2">
				<p class="min-w-0 text-sm break-all">
					<span class="font-medium">{upload.name}</span>
					<span class="text-muted-foreground">· {size(upload.size)}</span>
				</p>
				<Button type="button" variant="outline" size="sm" onclick={clearUpload}>
					Type it instead
				</Button>
			</div>
		{:else}
			<Textarea
				{id}
				bind:value={typed}
				rows={6}
				spellcheck={false}
				class="font-mono text-xs"
				aria-describedby="{id}-hint"
			/>
		{/if}
		<Field.Description id="{id}-hint">
			Type the text, or upload a file of any kind, up to {size(FILE_LIMIT)}.
		</Field.Description>
	</Field.Field>
	<Field.Field>
		<Field.Label for="{id}-upload">Upload a file</Field.Label>
		<Input id="{id}-upload" type="file" bind:value={chosen} onchange={picked} />
		{#if tooBig}
			<Field.Error>{tooBig}</Field.Error>
		{/if}
	</Field.Field>
{/if}
