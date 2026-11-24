<script lang="ts">
	import { page } from '$app/state';
	import AppSidebar from '#lib/components/app-sidebar.svelte';
	import BrandMark from '#lib/components/brand-mark.svelte';
	import ThemeToggle from '#lib/components/theme-toggle.svelte';
	import { SIDEBAR_COOKIE_NAME } from '#lib/components/ui/sidebar/constants.js';
	import * as Sidebar from '#lib/components/ui/sidebar/index.js';
	import { currentItem, navigation } from '#lib/navigation.js';
	import type { LayoutProps } from './$types';

	let { data, children }: LayoutProps = $props();

	const open = !document.cookie.split('; ').includes(`${SIDEBAR_COOKIE_NAME}=false`);

	const items = $derived(navigation(data.user, data.providers));
	const section = $derived(currentItem(items, page.url.pathname));
</script>

<Sidebar.Provider {open}>
	<AppSidebar user={data.user} navigation={items} />
	<Sidebar.Inset>
		<header
			class="sticky top-0 z-10 flex h-14 shrink-0 items-center gap-2 border-b bg-background/95 px-4 backdrop-blur"
		>
			<Sidebar.Trigger class="-ml-1" />
			<a href="/sandboxes" class="md:hidden" aria-label="Corral">
				<BrandMark class="size-6 rounded-md [&_svg]:size-3.5" />
			</a>
			<p class="truncate font-semibold md:hidden">{section?.label ?? 'Corral'}</p>
			<div class="ml-auto">
				<ThemeToggle />
			</div>
		</header>
		<main class="mx-auto w-full max-w-5xl flex-1 p-4 md:p-8">
			{@render children()}
		</main>
	</Sidebar.Inset>
</Sidebar.Provider>
