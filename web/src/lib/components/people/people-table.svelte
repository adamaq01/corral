<script lang="ts" module>
	import type { LocalAccount, User } from '#lib/api/generated/index.js';

	/** A dialog a row's menu opens. */
	export type RowDialog =
		| { dialog: 'disable' | 'allowance' | 'delete-user'; user: User }
		| { dialog: 'reset-password' | 'delete-account'; account: LocalAccount };
</script>

<script lang="ts">
	import EllipsisIcon from '@lucide/svelte/icons/ellipsis';
	import { sandboxesOf } from '#lib/all-sandboxes.js';
	import type { Role } from '#lib/api/generated/index.js';
	import TableSkeleton from '#lib/components/table-skeleton.svelte';
	import Timestamp from '#lib/components/timestamp.svelte';
	import { Badge, type BadgeVariant } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import { Spinner } from '#lib/components/ui/spinner/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import UserAvatar from '#lib/components/user-avatar.svelte';
	import type { Person } from '#lib/people.js';

	let {
		people,
		enabling,
		onopen,
		onenable
	}: {
		/** `undefined` while they load. */
		people: Person[] | undefined;
		/** The ids of the users whose enabling is in flight. */
		enabling: ReadonlySet<string>;
		onopen: (opened: RowDialog) => void;
		onenable: (user: User) => void;
	} = $props();

	const statuses: Record<Person['status'], { label: string; variant: BadgeVariant }> = {
		active: { label: 'Active', variant: 'secondary' },
		disabled: { label: 'Disabled', variant: 'outline' },
		deleting: { label: 'Deleting', variant: 'destructive' },
		waiting: { label: 'Not signed in yet', variant: 'outline' }
	};

	const roles: Record<Role, string> = { admin: 'Admin', member: 'Member' };

	const wide = 'hidden sm:table-cell';
	const wider = 'hidden lg:table-cell';
	const columns = ['', wider, wide, '', wide, wider, ''];
</script>

<div class="rounded-lg border">
	<Table.Root aria-label="People" aria-busy={people === undefined}>
		<Table.Header>
			<Table.Row>
				<Table.Head>Person</Table.Head>
				<Table.Head class={wider}>Signs in with</Table.Head>
				<Table.Head class={wide}>Role</Table.Head>
				<Table.Head>Status</Table.Head>
				<Table.Head class={wide}>Sandboxes</Table.Head>
				<Table.Head class={wider}>Created</Table.Head>
				<Table.Head><span class="sr-only">Actions</span></Table.Head>
			</Table.Row>
		</Table.Header>
		<Table.Body>
			{#if people === undefined}
				<TableSkeleton {columns} />
			{:else}
				{#each people as person (person.user?.id ?? person.handle)}
					{@const status = statuses[person.status]}
					<Table.Row>
						<Table.Cell>
							<div class="flex items-center gap-3">
								<UserAvatar handle={person.handle} picture={person.picture} />
								<div class="grid min-w-0">
									<span class="flex items-center gap-2 font-medium">
										{#if person.user}
											<a
												href={sandboxesOf(person.handle)}
												class="rounded-sm underline decoration-muted-foreground/50 underline-offset-4 hover:decoration-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none"
												title="Sandboxes of {person.handle}"
											>
												{person.handle}
											</a>
										{:else}
											{person.handle}
										{/if}
										{#if person.self}
											<Badge variant="outline">You</Badge>
										{/if}
									</span>
									<span class="text-xs text-muted-foreground sm:hidden">{roles[person.role]}</span>
									{#if person.email}
										<span
											class="max-w-40 truncate text-xs text-muted-foreground sm:max-w-none"
											title={person.email}
										>
											{person.email}
										</span>
									{/if}
								</div>
							</div>
						</Table.Cell>
						<Table.Cell class={wider}>{person.signsInWith}</Table.Cell>
						<Table.Cell class={wide}>{roles[person.role]}</Table.Cell>
						<Table.Cell>
							{#if person.user && enabling.has(person.user.id)}
								<Badge variant="outline">
									<Spinner role="presentation" aria-label={undefined} aria-hidden="true" />
									Enabling
								</Badge>
							{:else}
								<Badge variant={status.variant}>{status.label}</Badge>
							{/if}
						</Table.Cell>
						<Table.Cell class={wide}>up to {person.maxSandboxes}</Table.Cell>
						<Table.Cell class={wider}><Timestamp value={person.createdAt} /></Table.Cell>
						<Table.Cell class="w-0 text-right">
							{#if person.self}
								<Button variant="ghost" size="sm" href="/account">Account</Button>
							{:else}
								<DropdownMenu.Root>
									<DropdownMenu.Trigger>
										{#snippet child({ props })}
											<Button
												variant="ghost"
												size="icon"
												aria-label="Actions for {person.handle}"
												{...props}
											>
												<EllipsisIcon />
											</Button>
										{/snippet}
									</DropdownMenu.Trigger>
									<DropdownMenu.Content align="end">
										{@render menu(person)}
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

{#snippet menu(person: Person)}
	{#if person.user}
		{@const user = person.user}
		<DropdownMenu.Item>
			{#snippet child({ props })}
				<a href={sandboxesOf(user.handle)} {...props}> Their sandboxes </a>
			{/snippet}
		</DropdownMenu.Item>
		{#if user.status === 'active'}
			<DropdownMenu.Item onSelect={() => onopen({ dialog: 'disable', user })}>
				Disable user
			</DropdownMenu.Item>
		{:else if user.status === 'disabled'}
			<DropdownMenu.Item disabled={enabling.has(user.id)} onSelect={() => onenable(user)}>
				Enable user
			</DropdownMenu.Item>
		{/if}
	{/if}
	{#if person.account}
		{@const account = person.account}
		<DropdownMenu.Item onSelect={() => onopen({ dialog: 'reset-password', account })}>
			Reset password
		</DropdownMenu.Item>
	{/if}
	{#if person.user}
		{@const user = person.user}
		<DropdownMenu.Item onSelect={() => onopen({ dialog: 'allowance', user })}>
			Sandbox allowance…
		</DropdownMenu.Item>
		<DropdownMenu.Item
			variant="destructive"
			onSelect={() => onopen({ dialog: 'delete-user', user })}
		>
			{user.status === 'deleting' ? 'Finish deleting' : 'Delete user'}
		</DropdownMenu.Item>
	{:else}
		{@const account = person.account}
		<DropdownMenu.Item
			variant="destructive"
			onSelect={() => onopen({ dialog: 'delete-account', account })}
		>
			Delete account
		</DropdownMenu.Item>
	{/if}
{/snippet}
