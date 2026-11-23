<script lang="ts">
	import type { Fact } from '#lib/api/generated/index.js';

	let { facts }: { facts: Fact[] } = $props();
</script>

<dl class="grid divide-y">
	{#each facts as fact (fact.label)}
		<div class="grid gap-1 py-2.5 first:pt-0 last:pb-0 sm:grid-cols-[13rem_1fr] sm:gap-4">
			<dt class="text-muted-foreground">{fact.label}</dt>
			{#if fact.value === null}
				<dd class="text-muted-foreground italic">None</dd>
			{:else}
				<dd class="font-mono text-xs/5 wrap-anywhere">{fact.value}</dd>
			{/if}
		</div>
	{/each}
</dl>
