<script lang="ts">
	import AppSidebar from '#lib/components/app-sidebar.svelte';
	import BrandMark from '#lib/components/brand-mark.svelte';
	import ThemeToggle from '#lib/components/theme-toggle.svelte';
	import { SIDEBAR_COOKIE_NAME } from '#lib/components/ui/sidebar/constants.js';
	import * as Sidebar from '#lib/components/ui/sidebar/index.js';
	import { navigation } from '#lib/navigation.js';
	import type { LayoutProps } from './$types';

	let { data, children }: LayoutProps = $props();

	const open = !document.cookie.split('; ').includes(`${SIDEBAR_COOKIE_NAME}=false`);
</script>

<Sidebar.Provider {open}>
	<AppSidebar user={data.user} navigation={navigation(data.user, data.providers)} />
	<Sidebar.Inset>
		<header
			class="sticky top-0 z-10 flex h-14 shrink-0 items-center gap-2 border-b bg-background/95 px-4 backdrop-blur"
		>
			<Sidebar.Trigger class="-ml-1" />
			<a href="/sandboxes" class="flex items-center gap-2 font-semibold md:hidden">
				<BrandMark class="size-6 rounded-md [&_svg]:size-3.5" />
				Corral
			</a>
			<div class="ml-auto">
				<ThemeToggle />
			</div>
		</header>
		<main class="mx-auto w-full max-w-5xl flex-1 p-4 md:p-8">
			{@render children()}
		</main>
	</Sidebar.Inset>
</Sidebar.Provider>
