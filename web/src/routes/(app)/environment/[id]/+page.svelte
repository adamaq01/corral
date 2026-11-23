<script lang="ts">
	import EntryDetail from '#lib/components/entry-detail.svelte';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
</script>

{#key data.id}
	<EntryDetail id={data.id} />
{/key}
