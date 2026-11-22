<script lang="ts">
	import { afterNavigate } from '$app/navigation';
	import { page } from '$app/state';
	import BrandMark from '#lib/components/brand-mark.svelte';
	import * as Sidebar from '#lib/components/ui/sidebar/index.js';
	import UserMenu from '#lib/components/user-menu.svelte';
	import type { Me } from '#lib/api/generated/index.js';
	import { isCurrent, type NavItem, type Navigation } from '#lib/navigation.js';

	let { user, navigation }: { user: Me; navigation: Navigation } = $props();

	const sidebar = Sidebar.useSidebar();
	afterNavigate(() => sidebar.setOpenMobile(false));
</script>

{#snippet group(name: string, items: NavItem[], heading: boolean)}
	<nav aria-label={name}>
		<Sidebar.Group>
			{#if heading}
				<Sidebar.GroupLabel>{name}</Sidebar.GroupLabel>
			{/if}
			<Sidebar.GroupContent>
				<Sidebar.Menu>
					{#each items as item (item.href)}
						<Sidebar.MenuItem>
							<Sidebar.MenuButton
								isActive={isCurrent(item, page.url.pathname)}
								tooltipContent={item.label}
							>
								{#snippet child({ props })}
									<a
										href={item.href}
										aria-current={isCurrent(item, page.url.pathname) ? 'page' : undefined}
										{...props}
									>
										<item.icon />
										<span>{item.label}</span>
									</a>
								{/snippet}
							</Sidebar.MenuButton>
						</Sidebar.MenuItem>
					{/each}
				</Sidebar.Menu>
			</Sidebar.GroupContent>
		</Sidebar.Group>
	</nav>
{/snippet}

<Sidebar.Root collapsible="icon">
	<Sidebar.Header>
		<Sidebar.Menu>
			<Sidebar.MenuItem>
				<Sidebar.MenuButton size="lg">
					{#snippet child({ props })}
						<a href="/sandboxes" {...props}>
							<BrandMark />
							<span class="truncate text-base font-semibold">Corral</span>
						</a>
					{/snippet}
				</Sidebar.MenuButton>
			</Sidebar.MenuItem>
		</Sidebar.Menu>
	</Sidebar.Header>
	<Sidebar.Content>
		{@render group('Main', navigation.main, false)}
		{#if navigation.admin}
			{@render group('Admin', navigation.admin, true)}
		{/if}
	</Sidebar.Content>
	<Sidebar.Footer>
		<UserMenu {user} />
	</Sidebar.Footer>
	<Sidebar.Rail />
</Sidebar.Root>
