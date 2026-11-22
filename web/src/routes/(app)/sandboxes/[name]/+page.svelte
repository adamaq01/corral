<script lang="ts">
	import SandboxDetail from '#lib/components/sandbox-detail.svelte';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
</script>

{#key data.name}
	<SandboxDetail name={data.name} handle={data.user.handle} />
{/key}
