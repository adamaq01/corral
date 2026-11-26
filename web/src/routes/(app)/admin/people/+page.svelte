<script lang="ts">
	import EllipsisIcon from '@lucide/svelte/icons/ellipsis';
	import UsersIcon from '@lucide/svelte/icons/users';
	import { SvelteSet } from 'svelte/reactivity';
	import { toast } from 'svelte-sonner';
	import * as admin from '#lib/api/admin.js';
	import { failureMessage } from '#lib/api/client.js';
	import type { User, UserStatus } from '#lib/api/generated/index.js';
	import ConfirmDialog from '#lib/components/confirm-dialog.svelte';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import AccountsManager from '#lib/components/people/accounts-manager.svelte';
	import PageHeader from '#lib/components/page-header.svelte';
	import TableSkeleton from '#lib/components/table-skeleton.svelte';
	import Timestamp from '#lib/components/timestamp.svelte';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import { Badge, type BadgeVariant } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import * as Empty from '#lib/components/ui/empty/index.js';
	import { Spinner } from '#lib/components/ui/spinner/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import UserAvatar from '#lib/components/user-avatar.svelte';
	import { Remote } from '#lib/remote.svelte.js';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	const accountsUrl = $derived(
		data.providers.find((provider) => provider.accounts !== null)?.accounts
	);

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
	let deleting = $state(false);
	let disabling = $state(false);
	/** The users whose enabling is in flight. */
	const enabling = new SvelteSet<string>();

	async function setEnabled(user: User, enabled: boolean) {
		const changed = await admin.setEnabled(user, enabled);
		users.update((list) => list.map((each) => (each.id === changed.id ? changed : each)));
		toast.success(enabled ? `Enabled ${user.handle}` : `Disabled ${user.handle}`);
	}

	async function enable(user: User) {
		failure = null;
		enabling.add(user.id);
		try {
			await setEnabled(user, true);
		} catch (error) {
			failure = `Enabling ${user.handle} failed: ${failureMessage(error)}`;
		} finally {
			enabling.delete(user.id);
		}
	}

	function confirm(user: User, action: 'disable' | 'delete') {
		failure = null;
		target = user;
		disabling = action === 'disable';
		deleting = action === 'delete';
	}

	async function remove(user: User) {
		try {
			await admin.deleteUser(user);
			toast.success(`Deleted ${user.handle}`);
		} finally {
			await users.reload();
		}
	}
</script>

<div class="space-y-6">
	<PageHeader title="People" description="Everyone who has signed in to Corral." />

	{#if failure}
		<Alert.Root variant="destructive">
			<Alert.Description class="first-letter:uppercase">{failure}</Alert.Description>
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
												<a
													href="/admin/sandboxes?owner={encodeURIComponent(user.handle)}"
													class="rounded-sm underline decoration-muted-foreground/50 underline-offset-4 hover:decoration-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none"
													title="Sandboxes of {user.handle}"
												>
													{user.handle}
												</a>
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
								<Table.Cell>
									{#if enabling.has(user.id)}
										<Badge variant="outline">
											<Spinner role="presentation" aria-label={undefined} aria-hidden="true" />
											Enabling
										</Badge>
									{:else}
										<Badge variant={status.variant}>{status.label}</Badge>
									{/if}
								</Table.Cell>
								<Table.Cell class={wide}><Timestamp value={user.createdAt} /></Table.Cell>
								<Table.Cell class="w-0 text-right">
									{#if self}
										<Button variant="ghost" size="sm" href="/account">Account</Button>
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
												<DropdownMenu.Item>
													{#snippet child({ props })}
														<a
															href="/admin/sandboxes?owner={encodeURIComponent(user.handle)}"
															{...props}
														>
															Their sandboxes
														</a>
													{/snippet}
												</DropdownMenu.Item>
												{#if user.status === 'active'}
													<DropdownMenu.Item onSelect={() => confirm(user, 'disable')}>
														Disable user
													</DropdownMenu.Item>
												{:else if user.status === 'disabled'}
													<DropdownMenu.Item
														disabled={enabling.has(user.id)}
														onSelect={() => enable(user)}
													>
														Enable user
													</DropdownMenu.Item>
												{/if}
												<DropdownMenu.Item
													variant="destructive"
													onSelect={() => confirm(user, 'delete')}
												>
													{user.status === 'deleting' ? 'Finish deleting' : 'Delete user'}
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
		<p class="text-sm text-muted-foreground">
			A role comes from how each person signs in: their single sign-on groups, or their local
			account.
		</p>
	{/if}

	{#if accountsUrl}
		<AccountsManager url={accountsUrl} />
	{/if}
</div>

{#if target}
	{@const user = target}
	<ConfirmDialog
		bind:open={deleting}
		title="Delete {user.handle}?"
		action="Delete user"
		confirmName={user.handle}
		onconfirm={() => remove(user)}
	>
		This deletes {user.handle} with their sandboxes, SSH keys and environment. It cannot be undone.
	</ConfirmDialog>
	<ConfirmDialog
		bind:open={disabling}
		title="Disable {user.handle}?"
		action="Disable user"
		onconfirm={() => setEnabled(user, false)}
	>
		<p>
			{user.handle} can no longer sign in or open new SSH connections, and their open sessions in Corral
			end.
		</p>
		<p>
			Their sandboxes, SSH keys and environment stay. Sandboxes running now keep running until they
			go to sleep. You can enable {user.handle} again at any time.
		</p>
	</ConfirmDialog>
{/if}
