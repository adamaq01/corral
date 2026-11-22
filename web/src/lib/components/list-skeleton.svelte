<script lang="ts">
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';

	let { what }: { what: string } = $props();
</script>

<div class="grid gap-2" role="status" aria-label="Loading {what}">
	{#each [0, 1] as row (row)}
		<Skeleton class="h-[66px] rounded-lg" />
	{/each}
</div>
