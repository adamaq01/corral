<script lang="ts">
	import AdminSandboxDetail from '#lib/components/admin-sandbox-detail.svelte';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
</script>

{#key `${data.handle}/${data.name}`}
	<AdminSandboxDetail handle={data.handle} name={data.name} />
{/key}
