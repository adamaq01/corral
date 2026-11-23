<script lang="ts">
	import type { EntryKind, StoredValue } from '#lib/api/generated/index.js';
	import Timestamp from '#lib/components/timestamp.svelte';
	import { summary } from '#lib/environment.js';

	let { value, kind }: { value: StoredValue; kind: EntryKind } = $props();

	const SHOWN = 4000;
	const shownText = $derived(value.shown === 'text' ? value.text.slice(0, SHOWN) : '');
</script>

<div class="grid min-w-0 gap-1">
	{#if value.shown === 'text' && kind === 'file'}
		<pre
			class="rounded-md bg-muted px-3 py-2 font-mono text-xs break-all whitespace-pre-wrap">{shownText ||
				' '}</pre>
		{#if value.text.length > SHOWN}
			<p class="text-xs text-muted-foreground">
				The first {SHOWN} of {value.text.length} characters.
			</p>
		{/if}
	{:else}
		<p class={['text-sm break-all', value.shown === 'text' && 'font-mono']}>
			{summary(value)}
		</p>
	{/if}
	<p class="text-xs text-muted-foreground">
		{value.shown === 'masked' || value.shown === 'hidden' ? 'Secret, set' : 'Set'}
		<Timestamp value={value.setAt} />
	</p>
</div>
