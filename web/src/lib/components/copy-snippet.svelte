<script lang="ts">
	import CheckIcon from '@lucide/svelte/icons/check';
	import CopyIcon from '@lucide/svelte/icons/copy';
	import { copyText } from '#lib/clipboard.js';
	import { Button } from '#lib/components/ui/button/index.js';

	let { label, text }: { label: string; text: string } = $props();

	let outcome = $state<'copied' | 'failed' | null>(null);
	let reset: ReturnType<typeof setTimeout> | undefined;

	async function copy() {
		try {
			await copyText(text);
			outcome = 'copied';
		} catch {
			outcome = 'failed';
		}
		clearTimeout(reset);
		reset = setTimeout(() => (outcome = null), 2000);
	}
</script>

<figure class="grid min-w-0 grid-cols-1 gap-1.5">
	<figcaption class="flex items-center justify-between gap-2 text-sm font-medium">
		{label}
		<span class="text-xs font-normal text-muted-foreground" aria-live="polite">
			{#if outcome === 'copied'}
				Copied
			{:else if outcome === 'failed'}
				Copying failed: select the text and copy it
			{/if}
		</span>
	</figcaption>
	<div class="flex items-start gap-2 rounded-lg border bg-muted/50 py-1 pr-1 pl-3">
		<pre
			class="min-w-0 flex-1 py-1.5 font-mono text-xs leading-5 break-all whitespace-pre-wrap"><code
				>{text}</code
			></pre>
		<Button variant="ghost" size="icon-sm" aria-label="Copy {label}" title="Copy" onclick={copy}>
			{#if outcome === 'copied'}
				<CheckIcon />
			{:else}
				<CopyIcon />
			{/if}
		</Button>
	</div>
</figure>
