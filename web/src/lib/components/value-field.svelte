<script lang="ts">
	import EyeIcon from '@lucide/svelte/icons/eye';
	import EyeOffIcon from '@lucide/svelte/icons/eye-off';
	import UploadIcon from '@lucide/svelte/icons/upload';
	import XIcon from '@lucide/svelte/icons/x';
	import { untrack } from 'svelte';
	import type { EntryKind, StoredValue, ValueInput } from '#lib/api/generated/index.js';
	import { FILE_LIMIT } from '#lib/api/generated/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Field from '#lib/components/ui/field/index.js';
	import * as InputGroup from '#lib/components/ui/input-group/index.js';
	import {
		characterCount,
		fileTooLarge,
		fileValue,
		size,
		utf8Size,
		variableTooLarge
	} from '#lib/environment.js';

	let {
		id,
		kind,
		secret,
		name = '',
		current = null,
		value = $bindable(null),
		label = 'Value'
	}: {
		id: string;
		kind: EntryKind;
		secret: boolean;
		/** The variable's name, which counts toward its size. */
		name?: string;
		/** The value this one replaces, if any: a plain text one is the field's starting text. */
		current?: StoredValue | null;
		/** What the field holds, ready to send, or `null` until it holds a valid change. */
		value: ValueInput | null;
		label?: string;
	} = $props();

	const initial = untrack(() => (current?.shown === 'text' ? current.text : ''));
	let typed = $state(initial);
	let shown = $state(false);
	let upload = $state<{ name: string; size: number; value: ValueInput } | null>(null);
	let tooBig = $state<string | null>(null);
	let pasted = $state<string | null>(null);
	let chosen = $state('');
	let picker = $state<HTMLInputElement | null>(null);

	const masked = $derived(secret && !shown);
	const problem = $derived(
		kind === 'variable' ? variableTooLarge(name, typed) : fileTooLarge(utf8Size(typed))
	);
	const unchanged = $derived(typed === initial && (current !== null || typed === ''));
	const hint = $derived(currently(current));

	$effect(() => {
		if (upload) {
			value = upload.value;
		} else {
			value = unchanged || problem ? null : { text: typed };
		}
	});

	function currently(value: StoredValue | null): string | null {
		switch (value?.shown) {
			case 'masked':
				return `Now secret, ${characterCount(value.length)}.`;
			case 'hidden':
				return `Now secret, ${size(value.size)}.`;
			case 'binary':
				return `Now binary, ${size(value.size)}.`;
			default:
				return null;
		}
	}

	function onpaste(event: ClipboardEvent) {
		const text = event.clipboardData?.getData('text') ?? '';
		pasted = null;
		if (/[\r\n]/.test(text.replace(/[\r\n]+$/, ''))) {
			event.preventDefault();
			const lines = text.split(/\r?\n/).length;
			pasted = `The pasted text has ${lines} lines, and a variable holds one. Make it a file instead, or paste it encoded, such as in base64.`;
		} else if (/[\r\n]+$/.test(text)) {
			event.preventDefault();
			const input = event.currentTarget as HTMLInputElement;
			input.setRangeText(
				text.replace(/[\r\n]+$/, ''),
				input.selectionStart ?? 0,
				input.selectionEnd ?? 0,
				'end'
			);
			typed = input.value;
		}
	}

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

{#snippet eye()}
	<InputGroup.Button
		size="icon-xs"
		aria-label={shown ? 'Hide the value' : 'Show the value'}
		aria-pressed={shown}
		aria-controls={id}
		onclick={() => (shown = !shown)}
	>
		{#if shown}
			<EyeOffIcon />
		{:else}
			<EyeIcon />
		{/if}
	</InputGroup.Button>
{/snippet}

{#if kind === 'variable'}
	<Field.Field data-invalid={problem || pasted ? true : undefined}>
		<Field.Label for={id}>{label}</Field.Label>
		<InputGroup.Root>
			<InputGroup.Input
				{id}
				type={masked ? 'password' : 'text'}
				bind:value={typed}
				{onpaste}
				oninput={() => (pasted = null)}
				autocomplete="off"
				autocapitalize="off"
				spellcheck={false}
				aria-invalid={problem || pasted ? true : undefined}
				aria-describedby="{id}-hint"
				class="font-mono"
			/>
			{#if secret}
				<InputGroup.Addon align="inline-end">{@render eye()}</InputGroup.Addon>
			{/if}
		</InputGroup.Root>
		{#if pasted || problem}
			<Field.Error id="{id}-hint">{pasted ?? problem}</Field.Error>
		{:else}
			<Field.Description id="{id}-hint">
				{[hint, 'One line of text.'].filter(Boolean).join(' ')}
			</Field.Description>
		{/if}
	</Field.Field>
{:else}
	<Field.Field data-invalid={problem || tooBig ? true : undefined}>
		<div class="flex items-center justify-between gap-2">
			<Field.Label for={id}>{label}</Field.Label>
			<input
				bind:this={picker}
				id="{id}-upload"
				type="file"
				class="sr-only"
				tabindex="-1"
				aria-label="Upload a file"
				bind:value={chosen}
				onchange={picked}
			/>
			<Button type="button" variant="ghost" size="sm" onclick={() => picker?.click()}>
				<UploadIcon />
				Upload a file
			</Button>
		</div>
		{#if upload}
			<div class="flex flex-wrap items-center justify-between gap-2 rounded-md border px-3 py-2">
				<p class="min-w-0 text-sm break-all">
					<span class="font-medium">{upload.name}</span>
					<span class="text-muted-foreground">· {size(upload.size)}</span>
				</p>
				<Button type="button" variant="ghost" size="sm" onclick={clearUpload}>
					<XIcon />
					Type it instead
				</Button>
			</div>
		{:else}
			<InputGroup.Root>
				<InputGroup.Textarea
					{id}
					bind:value={typed}
					rows={6}
					spellcheck={false}
					autocomplete="off"
					aria-invalid={problem ? true : undefined}
					aria-describedby="{id}-hint"
					class={[
						'max-h-80 font-mono text-xs',
						masked && '[-webkit-text-security:disc] [text-security:disc]'
					]}
				/>
				{#if secret}
					<InputGroup.Addon align="block-end" class="justify-end">{@render eye()}</InputGroup.Addon>
				{/if}
			</InputGroup.Root>
		{/if}
		{#if tooBig}
			<Field.Error id="{id}-hint">{tooBig}</Field.Error>
		{:else if problem && !upload}
			<Field.Error id="{id}-hint">{problem}</Field.Error>
		{:else}
			<Field.Description id="{id}-hint">
				{[hint, `Type the text, or upload a file of any kind, up to ${size(FILE_LIMIT)}.`]
					.filter(Boolean)
					.join(' ')}
			</Field.Description>
		{/if}
	</Field.Field>
{/if}
