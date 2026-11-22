<script lang="ts">
	import type { Snippet } from 'svelte';

	let { title, description, actions }: { title: string; description: string; actions?: Snippet } =
		$props();
</script>

<svelte:head>
	<title>{title} · Corral</title>
</svelte:head>

<header class="flex flex-wrap items-center justify-between gap-x-6 gap-y-4">
	<div class="grid gap-1">
		<h1 class="text-2xl font-semibold tracking-tight">{title}</h1>
		<p class="text-sm text-muted-foreground">{description}</p>
	</div>
	{@render actions?.()}
</header>
