<script lang="ts">
	import { page } from '$app/state';
	import PageHeader from '#lib/components/page-header.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import type { LayoutProps } from './$types';

	let { children }: LayoutProps = $props();

	const tabs = [
		{ href: '/admin/settings/catalog', label: 'Catalog' },
		{ href: '/admin/settings/limits', label: 'Limits' },
		{ href: '/admin/settings/registries', label: 'Registries' }
	];
</script>

<div class="space-y-6">
	<PageHeader title="Settings" description="What this Corral offers its members." />

	<nav aria-label="Settings">
		<ul class="flex flex-wrap gap-1">
			{#each tabs as tab (tab.href)}
				{@const current = page.url.pathname === tab.href}
				<li>
					<Button
						variant={current ? 'secondary' : 'ghost'}
						href={tab.href}
						aria-current={current ? 'page' : undefined}
					>
						{tab.label}
					</Button>
				</li>
			{/each}
		</ul>
	</nav>

	{@render children()}
</div>
