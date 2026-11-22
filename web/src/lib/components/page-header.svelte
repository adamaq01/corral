<script lang="ts">
	let { title, description }: { title: string; description: string } = $props();
</script>

<svelte:head>
	<title>{title} · Corral</title>
</svelte:head>

<header class="space-y-1">
	<h1 class="text-2xl font-semibold tracking-tight">{title}</h1>
	<p class="text-sm text-muted-foreground">{description}</p>
</header>
