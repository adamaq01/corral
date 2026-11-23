<script lang="ts">
	import EllipsisIcon from '@lucide/svelte/icons/ellipsis';
	import UsersIcon from '@lucide/svelte/icons/users';
	import * as admin from '#lib/api/admin.js';
	import { failureMessage } from '#lib/api/client.js';
	import type { User, UserStatus } from '#lib/api/generated/index.js';
	import ConfirmDialog from '#lib/components/confirm-dialog.svelte';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import PageHeader from '#lib/components/page-header.svelte';
	import TableSkeleton from '#lib/components/table-skeleton.svelte';
	import Timestamp from '#lib/components/timestamp.svelte';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import { Badge, type BadgeVariant } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import * as Empty from '#lib/components/ui/empty/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import UserAvatar from '#lib/components/user-avatar.svelte';
	import { Remote } from '#lib/remote.svelte.js';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	const statuses: Record<UserStatus, { label: string; variant: BadgeVariant }> = {
		active: { label: 'Active', variant: 'secondary' },
		disabled: { label: 'Disabled', variant: 'outline' },
		deleting: { label: 'Deleting', variant: 'destructive' }
	};

	const wide = 'hidden sm:table-cell';
	const columns = ['', wide, '', wide, ''];

	const users = new Remote(admin.users);

	let failure = $state<string | null>(null);
	let target = $state<User | null>(null);
	let confirming = $state(false);

	async function setEnabled(user: User, enabled: boolean) {
		failure = null;
		try {
			const changed = await admin.setEnabled(user, enabled);
			users.update((list) => list.map((each) => (each.id === changed.id ? changed : each)));
		} catch (error) {
			const verb = enabled ? 'Enabling' : 'Disabling';
			failure = `${verb} ${user.handle} failed: ${failureMessage(error)}`;
		}
	}

	function confirmDelete(user: User) {
		failure = null;
		target = user;
		confirming = true;
	}

	async function remove(user: User) {
		await admin.deleteUser(user);
		users.update((list) => list.filter((each) => each.id !== user.id));
	}
</script>

<div class="space-y-6">
	<PageHeader title="Users" description="Everyone who has signed in to Corral." />

	{#if failure}
		<Alert.Root variant="destructive">
			<Alert.Description>{failure}</Alert.Description>
		</Alert.Root>
	{/if}

	{#if users.current.state === 'failed'}
		<LoadFailed
			title="Loading the users failed"
			error={users.current.error}
			onretry={() => users.reload()}
		/>
	{:else if users.current.state === 'ready' && users.current.value.length === 0}
		<Empty.Root class="border">
			<Empty.Header>
				<Empty.Media variant="icon"><UsersIcon /></Empty.Media>
				<Empty.Title>No users yet</Empty.Title>
				<Empty.Description>People appear here once they first sign in.</Empty.Description>
			</Empty.Header>
		</Empty.Root>
	{:else}
		<div class="rounded-lg border">
			<Table.Root aria-label="Users" aria-busy={users.current.state === 'loading'}>
				<Table.Header>
					<Table.Row>
						<Table.Head>User</Table.Head>
						<Table.Head class={wide}>Role</Table.Head>
						<Table.Head>Status</Table.Head>
						<Table.Head class={wide}>Created</Table.Head>
						<Table.Head><span class="sr-only">Actions</span></Table.Head>
					</Table.Row>
				</Table.Header>
				<Table.Body>
					{#if users.current.state === 'loading'}
						<TableSkeleton {columns} />
					{:else}
						{#each users.current.value as user (user.id)}
							{@const self = user.handle === data.user.handle}
							{@const status = statuses[user.status]}
							{@const role = user.role === 'admin' ? 'Admin' : 'Member'}
							<Table.Row>
								<Table.Cell>
									<div class="flex items-center gap-3">
										<UserAvatar handle={user.handle} picture={user.picture} />
										<div class="grid min-w-0">
											<span class="flex items-center gap-2 font-medium">
												{user.handle}
												{#if self}
													<Badge variant="outline">You</Badge>
												{/if}
											</span>
											<span class="text-xs text-muted-foreground sm:hidden">{role}</span>
											{#if user.email}
												<span
													class="max-w-40 truncate text-xs text-muted-foreground sm:max-w-none"
													title={user.email}
												>
													{user.email}
												</span>
											{/if}
										</div>
									</div>
								</Table.Cell>
								<Table.Cell class={wide}>{role}</Table.Cell>
								<Table.Cell><Badge variant={status.variant}>{status.label}</Badge></Table.Cell>
								<Table.Cell class={wide}><Timestamp value={user.createdAt} /></Table.Cell>
								<Table.Cell class="w-0 text-right">
									{#if self}
										<Button variant="ghost" size="sm" href="/profile">Profile</Button>
									{:else}
										<DropdownMenu.Root>
											<DropdownMenu.Trigger>
												{#snippet child({ props })}
													<Button
														variant="ghost"
														size="icon"
														aria-label="Actions for {user.handle}"
														{...props}
													>
														<EllipsisIcon />
													</Button>
												{/snippet}
											</DropdownMenu.Trigger>
											<DropdownMenu.Content align="end">
												{#if user.status === 'active'}
													<DropdownMenu.Item onSelect={() => setEnabled(user, false)}>
														Disable
													</DropdownMenu.Item>
												{:else if user.status === 'disabled'}
													<DropdownMenu.Item onSelect={() => setEnabled(user, true)}>
														Enable
													</DropdownMenu.Item>
												{/if}
												<DropdownMenu.Item
													variant="destructive"
													onSelect={() => confirmDelete(user)}
												>
													{user.status === 'deleting' ? 'Finish deleting' : 'Delete'}
												</DropdownMenu.Item>
											</DropdownMenu.Content>
										</DropdownMenu.Root>
									{/if}
								</Table.Cell>
							</Table.Row>
						{/each}
					{/if}
				</Table.Body>
			</Table.Root>
		</div>
	{/if}
</div>

{#if target}
	{@const user = target}
	<ConfirmDialog
		bind:open={confirming}
		title="Delete {user.handle}?"
		action="Delete user"
		onconfirm={() => remove(user)}
	>
		This deletes {user.handle} with their sandboxes, SSH keys and environment. It cannot be undone.
	</ConfirmDialog>
{/if}
