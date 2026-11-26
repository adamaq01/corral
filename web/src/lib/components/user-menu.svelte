<script lang="ts">
	import ChevronsUpDownIcon from '@lucide/svelte/icons/chevrons-up-down';
	import CircleUserRoundIcon from '@lucide/svelte/icons/circle-user-round';
	import LogOutIcon from '@lucide/svelte/icons/log-out';
	import { toast } from 'svelte-sonner';
	import { goto } from '$app/navigation';
	import { signOut } from '#lib/api/auth.js';
	import { failureMessage } from '#lib/api/client.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import * as Sidebar from '#lib/components/ui/sidebar/index.js';
	import UserAvatar from '#lib/components/user-avatar.svelte';
	import type { Me } from '#lib/api/generated/index.js';

	let { user }: { user: Me } = $props();

	const sidebar = Sidebar.useSidebar();

	async function leave() {
		try {
			await signOut();
		} catch (error) {
			// The menu that held the button is gone, so the failure has nowhere else to show.
			toast.error(`Signing out failed: ${failureMessage(error)}`);
			return;
		}
		await goto('/sign-in', { replace: true });
	}
</script>

{#snippet identity()}
	<UserAvatar handle={user.handle} picture={user.picture} />
	<div class="grid flex-1 text-left text-sm leading-tight">
		<span class="truncate font-medium">{user.handle}</span>
		{#if user.email}
			<span class="truncate text-xs text-muted-foreground">{user.email}</span>
		{/if}
	</div>
{/snippet}

<Sidebar.Menu>
	<Sidebar.MenuItem>
		<DropdownMenu.Root>
			<DropdownMenu.Trigger>
				{#snippet child({ props })}
					<Sidebar.MenuButton
						size="lg"
						class="data-[state=open]:bg-sidebar-accent data-[state=open]:text-sidebar-accent-foreground"
						{...props}
					>
						<span class="sr-only">Account</span>
						{@render identity()}
						<ChevronsUpDownIcon class="ml-auto" />
					</Sidebar.MenuButton>
				{/snippet}
			</DropdownMenu.Trigger>
			<DropdownMenu.Content
				class="w-(--bits-dropdown-menu-anchor-width) min-w-56"
				side={sidebar.isMobile ? 'bottom' : 'right'}
				align="end"
				sideOffset={4}
			>
				<DropdownMenu.Label class="p-0 font-normal text-foreground">
					<div class="flex items-center gap-2 px-1 py-1.5">
						{@render identity()}
					</div>
				</DropdownMenu.Label>
				<DropdownMenu.Separator />
				<DropdownMenu.Item>
					{#snippet child({ props })}
						<a href="/account" {...props}>
							<CircleUserRoundIcon />
							Account
						</a>
					{/snippet}
				</DropdownMenu.Item>
				<DropdownMenu.Separator />
				<DropdownMenu.Item onSelect={leave}>
					<LogOutIcon />
					Sign out
				</DropdownMenu.Item>
			</DropdownMenu.Content>
		</DropdownMenu.Root>
	</Sidebar.MenuItem>
</Sidebar.Menu>
