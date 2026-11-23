<script lang="ts">
	import CircleCheckIcon from '@lucide/svelte/icons/circle-check';
	import EllipsisIcon from '@lucide/svelte/icons/ellipsis';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import UserCogIcon from '@lucide/svelte/icons/user-cog';
	import { accounts } from '#lib/api/accounts.js';
	import { MIN_PASSWORD_CHARS, type LocalAccount, type Role } from '#lib/api/generated/index.js';
	import ConfirmDialog from '#lib/components/confirm-dialog.svelte';
	import FormDialog from '#lib/components/form-dialog.svelte';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import PageHeader from '#lib/components/page-header.svelte';
	import TableSkeleton from '#lib/components/table-skeleton.svelte';
	import Timestamp from '#lib/components/timestamp.svelte';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import * as Empty from '#lib/components/ui/empty/index.js';
	import * as Field from '#lib/components/ui/field/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import * as RadioGroup from '#lib/components/ui/radio-group/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import { Remote } from '#lib/remote.svelte.js';

	/** The `accounts` URL of the provider that keeps them. */
	let { url }: { url: string } = $props();

	const roles: { value: Role; label: string; description: string }[] = [
		{
			value: 'member',
			label: 'Member',
			description: 'Manages their own sandboxes, SSH keys and environment.'
		},
		{
			value: 'admin',
			label: 'Admin',
			description: 'Also manages users and local accounts, and sees every sandbox.'
		}
	];

	const created = 'hidden sm:table-cell';
	const columns = ['', '', created, ''];

	const api = $derived(accounts(url));
	const list = new Remote(() => api.list());

	let notice = $state<string | null>(null);

	let creating = $state(false);
	let username = $state('');
	let password = $state('');
	let role = $state<Role>('member');

	let target = $state<LocalAccount | null>(null);
	let resetting = $state(false);
	let deleting = $state(false);
	let newPassword = $state('');

	function startCreating() {
		notice = null;
		username = '';
		password = '';
		role = 'member';
		creating = true;
	}

	async function create() {
		await api.create({ username, password, role });
		await list.reload();
		notice = `Created ${username}, who can sign in now.`;
	}

	function startResetting(account: LocalAccount) {
		notice = null;
		target = account;
		newPassword = '';
		resetting = true;
	}

	async function reset(account: LocalAccount) {
		await api.setPassword(account.username, newPassword);
		notice = account.yours
			? 'Changed your password.'
			: `Reset the password of ${account.username}.`;
	}

	function startDeleting(account: LocalAccount) {
		notice = null;
		target = account;
		deleting = true;
	}

	async function remove(account: LocalAccount) {
		await api.remove(account.username);
		list.update((all) => all.filter((each) => each.username !== account.username));
		notice = `Deleted ${account.username}.`;
	}
</script>

{#snippet newAccountButton()}
	<Button onclick={startCreating}><PlusIcon />New account</Button>
{/snippet}

<div class="space-y-6">
	<PageHeader title="Local accounts" description="The accounts that sign in with a password.">
		{#snippet actions()}
			{#if list.current.state === 'ready' && list.current.value.length > 0}
				{@render newAccountButton()}
			{/if}
		{/snippet}
	</PageHeader>

	{#if notice}
		<Alert.Root role="status">
			<CircleCheckIcon />
			<Alert.Description>{notice}</Alert.Description>
		</Alert.Root>
	{/if}

	{#if list.current.state === 'failed'}
		<LoadFailed
			title="Loading the local accounts failed"
			error={list.current.error}
			onretry={() => list.reload()}
		/>
	{:else if list.current.state === 'ready' && list.current.value.length === 0}
		<Empty.Root class="border">
			<Empty.Header>
				<Empty.Media variant="icon"><UserCogIcon /></Empty.Media>
				<Empty.Title>No local accounts</Empty.Title>
				<Empty.Description>
					Create one for each person who signs in with a username and password.
				</Empty.Description>
			</Empty.Header>
			<Empty.Content>{@render newAccountButton()}</Empty.Content>
		</Empty.Root>
	{:else}
		<div class="rounded-lg border">
			<Table.Root aria-label="Local accounts" aria-busy={list.current.state === 'loading'}>
				<Table.Header>
					<Table.Row>
						<Table.Head>Username</Table.Head>
						<Table.Head>Role</Table.Head>
						<Table.Head class={created}>Created</Table.Head>
						<Table.Head><span class="sr-only">Actions</span></Table.Head>
					</Table.Row>
				</Table.Header>
				<Table.Body>
					{#if list.current.state === 'loading'}
						<TableSkeleton {columns} />
					{:else}
						{#each list.current.value as account (account.username)}
							<Table.Row>
								<Table.Cell class="font-medium">
									<span class="flex items-center gap-2">
										{account.username}
										{#if account.yours}
											<Badge variant="outline">You</Badge>
										{/if}
									</span>
								</Table.Cell>
								<Table.Cell>{account.role === 'admin' ? 'Admin' : 'Member'}</Table.Cell>
								<Table.Cell class={created}><Timestamp value={account.createdAt} /></Table.Cell>
								<Table.Cell class="w-0 text-right">
									<DropdownMenu.Root>
										<DropdownMenu.Trigger>
											{#snippet child({ props })}
												<Button
													variant="ghost"
													size="icon"
													aria-label="Actions for {account.username}"
													{...props}
												>
													<EllipsisIcon />
												</Button>
											{/snippet}
										</DropdownMenu.Trigger>
										<DropdownMenu.Content align="end">
											<DropdownMenu.Item onSelect={() => startResetting(account)}>
												{account.yours ? 'Change password' : 'Reset password'}
											</DropdownMenu.Item>
											{#if account.yours}
												<DropdownMenu.Item>
													{#snippet child({ props })}
														<a href="/profile" {...props}>Delete from Profile</a>
													{/snippet}
												</DropdownMenu.Item>
											{:else}
												<DropdownMenu.Item
													variant="destructive"
													onSelect={() => startDeleting(account)}
												>
													Delete
												</DropdownMenu.Item>
											{/if}
										</DropdownMenu.Content>
									</DropdownMenu.Root>
								</Table.Cell>
							</Table.Row>
						{/each}
					{/if}
				</Table.Body>
			</Table.Root>
		</div>
	{/if}
</div>

<FormDialog
	bind:open={creating}
	title="New local account"
	description="The person signs in with this username and password."
	submit="Create account"
	onsubmit={create}
>
	<Field.Field>
		<Field.Label for="new-username">Username</Field.Label>
		<Input
			id="new-username"
			autocomplete="off"
			autocapitalize="none"
			spellcheck={false}
			pattern={'[a-z0-9\\-]{1,32}'}
			required
			aria-describedby="new-username-hint"
			bind:value={username}
		/>
		<Field.Description id="new-username-hint">
			Up to 32 lowercase letters, digits and dashes. It becomes their handle.
		</Field.Description>
	</Field.Field>
	<Field.Field>
		<Field.Label for="new-password">Password</Field.Label>
		<Input
			id="new-password"
			type="password"
			autocomplete="new-password"
			minlength={MIN_PASSWORD_CHARS}
			required
			aria-describedby="new-password-hint"
			bind:value={password}
		/>
		<Field.Description id="new-password-hint">
			At least {MIN_PASSWORD_CHARS} characters.
		</Field.Description>
	</Field.Field>
	<Field.Set>
		<Field.Legend variant="label">Role</Field.Legend>
		<RadioGroup.Root bind:value={role}>
			{#each roles as choice (choice.value)}
				<Field.Field orientation="horizontal">
					<RadioGroup.Item
						value={choice.value}
						id="role-{choice.value}"
						aria-describedby="role-{choice.value}-hint"
					/>
					<Field.Content>
						<Field.Label for="role-{choice.value}">{choice.label}</Field.Label>
						<Field.Description id="role-{choice.value}-hint">
							{choice.description}
						</Field.Description>
					</Field.Content>
				</Field.Field>
			{/each}
		</RadioGroup.Root>
	</Field.Set>
</FormDialog>

{#if target}
	{@const account = target}
	<FormDialog
		bind:open={resetting}
		title={account.yours ? 'Change your password' : `Reset the password of ${account.username}`}
		description="{account.yours ? 'You sign' : 'They sign'} in with the new password from now on."
		submit={account.yours ? 'Change password' : 'Reset password'}
		onsubmit={() => reset(account)}
	>
		<Field.Field>
			<Field.Label for="reset-password">New password</Field.Label>
			<Input
				id="reset-password"
				type="password"
				autocomplete="new-password"
				minlength={MIN_PASSWORD_CHARS}
				required
				aria-describedby="reset-password-hint"
				bind:value={newPassword}
			/>
			<Field.Description id="reset-password-hint">
				At least {MIN_PASSWORD_CHARS} characters.
			</Field.Description>
		</Field.Field>
	</FormDialog>
	<ConfirmDialog
		bind:open={deleting}
		title="Delete {account.username}?"
		action="Delete account"
		onconfirm={() => remove(account)}
	>
		This deletes the account and the user who signs in with it, with their sandboxes, SSH keys and
		environment. It cannot be undone.
	</ConfirmDialog>
{/if}
