<script lang="ts">
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import * as Table from '#lib/components/ui/table/index.js';

	/** One class per column, so a column hidden at some widths is hidden here too. */
	let { columns, rows = 3 }: { columns: string[]; rows?: number } = $props();
</script>

{#each { length: rows }, row (row)}
	<Table.Row aria-hidden="true">
		{#each columns as column, index (index)}
			<Table.Cell class={column}>
				<Skeleton class="h-4 w-full max-w-24" />
			</Table.Cell>
		{/each}
	</Table.Row>
{/each}
