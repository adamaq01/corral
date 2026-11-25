<script lang="ts">
	import DownloadIcon from '@lucide/svelte/icons/download';
	import LockIcon from '@lucide/svelte/icons/lock';
	import type { EntryKind, StoredValue } from '#lib/api/generated/index.js';
	import CopyButton from '#lib/components/copy-button.svelte';
	import Timestamp from '#lib/components/timestamp.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { characterCount, download, size, valueBytes } from '#lib/environment.js';

	let {
		value,
		kind,
		label,
		file = 'value'
	}: {
		value: StoredValue;
		kind: EntryKind;
		/** What the value is the value of, as in "GH_TOKEN" or "~/.gitconfig in main". */
		label: string;
		/** The name a download takes. */
		file?: string;
	} = $props();

	/** Lines a plain text file shows before "Show all". */
	const LINES = 12;
	const DOTS = 12;

	let expanded = $state(false);
	const lines = $derived(value.shown === 'text' ? value.text.split('\n') : []);
	const long = $derived(lines.length > LINES);
	const shownText = $derived(
		expanded || !long ? lines.join('\n') : lines.slice(0, LINES).join('\n')
	);
	const bytes = $derived(valueBytes(value));
</script>

{#snippet secretBox(content: string, length: string)}
	<div
		class="flex min-h-9 min-w-0 items-center gap-2 rounded-md border bg-muted/40 px-3 py-1.5 font-mono text-sm"
		aria-label="Secret value, {length}"
		role="img"
	>
		<LockIcon class="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
		<span class="min-w-0 truncate tracking-wide">{content}</span>
		<span class="ml-auto shrink-0 font-sans text-xs text-muted-foreground">{length}</span>
	</div>
{/snippet}

<div class="grid min-w-0 gap-1.5">
	{#if value.shown === 'masked'}
		{@const dots = '•'.repeat(Math.min(value.length, DOTS))}
		{@render secretBox(
			value.head === null ? dots : `${value.head}${dots}${value.tail}`,
			characterCount(value.length)
		)}
	{:else if value.shown === 'hidden'}
		{@render secretBox('•'.repeat(DOTS), size(value.size))}
	{:else if value.shown === 'binary'}
		<div class="flex min-h-9 flex-wrap items-center gap-2 rounded-md border px-3 py-1.5 text-sm">
			<span>Binary, {size(value.size)}</span>
			{#if bytes}
				<Button
					variant="ghost"
					size="sm"
					class="ml-auto"
					onclick={() => bytes && download(file, bytes)}
				>
					<DownloadIcon />
					Download<span class="sr-only"> {label}</span>
				</Button>
			{/if}
		</div>
	{:else if kind === 'variable'}
		<div class="flex min-h-9 min-w-0 items-center gap-1 rounded-md border bg-muted/40 pl-3">
			<code class="min-w-0 flex-1 py-1.5 text-sm break-all">{value.text}</code>
			<CopyButton text={value.text} label="the value of {label}" class="shrink-0" />
		</div>
	{:else}
		<div class="relative min-w-0 rounded-md border bg-muted/40">
			<pre
				class="max-h-[32rem] overflow-auto px-3 py-2 pr-20 font-mono text-xs break-all whitespace-pre-wrap">{shownText ||
					' '}</pre>
			<div class="absolute top-1 right-1 flex">
				<CopyButton text={value.text} label="the text of {label}" />
				{#if bytes}
					<Button
						variant="ghost"
						size="icon-sm"
						aria-label="Download {label}"
						title="Download {label}"
						onclick={() => bytes && download(file, bytes)}
					>
						<DownloadIcon />
					</Button>
				{/if}
			</div>
		</div>
		{#if long}
			<Button
				variant="link"
				size="sm"
				class="h-auto w-fit p-0"
				aria-expanded={expanded}
				onclick={() => (expanded = !expanded)}
			>
				{expanded ? `Show the first ${LINES} lines` : `Show all ${lines.length} lines`}
			</Button>
		{/if}
	{/if}
	<p class="text-xs text-muted-foreground">
		{value.shown === 'masked' || value.shown === 'hidden' ? 'Secret, set' : 'Set'}
		<Timestamp value={value.setAt} />
	</p>
</div>
