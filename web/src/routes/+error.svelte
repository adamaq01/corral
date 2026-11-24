<script lang="ts">
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import SearchXIcon from '@lucide/svelte/icons/search-x';
	import { page } from '$app/state';
	import BrandMark from '#lib/components/brand-mark.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Empty from '#lib/components/ui/empty/index.js';

	const missing = $derived(page.status === 404);
</script>

<svelte:head>
	<title>{missing ? 'Not found' : 'Something went wrong'} · Corral</title>
</svelte:head>

<main class="flex min-h-svh flex-col items-center justify-center gap-6 bg-muted p-4 md:p-10">
	<a href="/sandboxes" class="flex items-center gap-2 text-lg font-semibold">
		<BrandMark />
		Corral
	</a>
	<Empty.Root class="w-full max-w-md flex-none border-solid bg-background">
		<Empty.Header>
			<Empty.Media variant="icon">
				{#if missing}
					<SearchXIcon />
				{:else}
					<CircleAlertIcon />
				{/if}
			</Empty.Media>
			<h1 class="text-base font-medium tracking-tight">
				{missing ? 'There is no such page' : 'Corral could not load this page'}
			</h1>
			<Empty.Description class="first-letter:uppercase">
				{missing ? 'The link may be out of date.' : page.error?.message}
			</Empty.Description>
		</Empty.Header>
		<Empty.Content class="flex-row flex-wrap justify-center">
			{#if !missing}
				<Button onclick={() => location.reload()}>Retry</Button>
			{/if}
			<Button href="/sandboxes" variant={missing ? 'default' : 'outline'}>Go to Sandboxes</Button>
		</Empty.Content>
	</Empty.Root>
</main>
