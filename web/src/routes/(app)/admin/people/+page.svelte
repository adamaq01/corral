<script lang="ts">
	import PlusIcon from '@lucide/svelte/icons/plus';
	import UsersIcon from '@lucide/svelte/icons/users';
	import { SvelteSet } from 'svelte/reactivity';
	import { toast } from 'svelte-sonner';
	import { accounts } from '#lib/api/accounts.js';
	import * as admin from '#lib/api/admin.js';
	import { failureMessage } from '#lib/api/client.js';
	import type { LocalAccount, User } from '#lib/api/generated/index.js';
	import * as settings from '#lib/api/settings.js';
	import ConfirmDialog from '#lib/components/confirm-dialog.svelte';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import PageHeader from '#lib/components/page-header.svelte';
	import AllowanceDialog from '#lib/components/people/allowance-dialog.svelte';
	import NewAccountDialog from '#lib/components/people/new-account-dialog.svelte';
	import PeopleTable, { type RowDialog } from '#lib/components/people/people-table.svelte';
	import ResetPasswordDialog from '#lib/components/people/reset-password-dialog.svelte';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Empty from '#lib/components/ui/empty/index.js';
	import { people, type Directory } from '#lib/people.js';
	import { Remote } from '#lib/remote.svelte.js';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	const accountsUrl = $derived(
		data.providers.find((provider) => provider.accounts !== null)?.accounts ?? null
	);

	const directory = new Remote(async (): Promise<Directory> => {
		const [users, listed, limits] = await Promise.all([
			admin.users(),
			accountsUrl === null ? [] : accounts(accountsUrl).list(),
			settings.limits()
		]);
		return { users, accounts: listed, limit: limits.maxSandboxesPerMember.value };
	});

	const rows = $derived(
		directory.value ? people(directory.value, data.providers, data.user.handle) : undefined
	);

	type Shown = RowDialog | { dialog: 'new-account' };

	/** The dialog shown, kept while it closes. */
	let shown = $state<Shown | null>(null);
	let open = $state(false);
	let failure = $state<string | null>(null);
	/** The ids of the users whose enabling is in flight. */
	const enabling = new SvelteSet<string>();

	function show(dialog: Shown) {
		failure = null;
		shown = dialog;
		open = true;
	}

	function changed(user: User) {
		directory.update((listed) => ({
			...listed,
			users: listed.users.map((each) => (each.id === user.id ? user : each))
		}));
	}

	async function setEnabled(user: User, enabled: boolean) {
		changed(await admin.setEnabled(user, enabled));
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

	function allowed(user: User) {
		changed(user);
		toast.success(
			user.allowance
				? `${user.handle} may have up to ${user.allowance.maxSandboxes} sandboxes`
				: `${user.handle} has the usual limit again`
		);
	}

	async function created(username: string) {
		toast.success(`Created ${username}, who can sign in now`);
		await directory.reload();
	}

	async function deleteUser(user: User) {
		try {
			await admin.deleteUser(user);
			toast.success(`Deleted ${user.handle}`);
		} finally {
			await directory.reload();
		}
	}

	async function deleteAccount(url: string, account: LocalAccount) {
		await accounts(url).remove(account.username);
		toast.success(`Deleted ${account.username}`);
		await directory.reload();
	}
</script>

{#snippet newAccount()}
	<Button onclick={() => show({ dialog: 'new-account' })}><PlusIcon />New local account</Button>
{/snippet}

<div class="space-y-6">
	<PageHeader title="People" description="Everyone who uses Corral, however they sign in.">
		{#snippet actions()}
			{#if accountsUrl && rows?.length}
				{@render newAccount()}
			{/if}
		{/snippet}
	</PageHeader>

	{#if failure}
		<Alert.Root variant="destructive">
			<Alert.Description class="first-letter:uppercase">{failure}</Alert.Description>
		</Alert.Root>
	{/if}

	{#if directory.current.state === 'failed'}
		<LoadFailed
			title="Loading the people failed"
			error={directory.current.error}
			onretry={() => directory.reload()}
		/>
	{:else if rows?.length === 0}
		<Empty.Root class="border">
			<Empty.Header>
				<Empty.Media variant="icon"><UsersIcon /></Empty.Media>
				<Empty.Title>No one yet</Empty.Title>
				<Empty.Description>People appear here once they first sign in.</Empty.Description>
			</Empty.Header>
			{#if accountsUrl}
				<Empty.Content>{@render newAccount()}</Empty.Content>
			{/if}
		</Empty.Root>
	{:else}
		<PeopleTable people={rows} {enabling} onopen={show} onenable={enable} />
		<p class="text-sm text-muted-foreground">
			A role comes from how each person signs in: their single sign-on groups, or their local
			account.
		</p>
	{/if}
</div>

{#if shown?.dialog === 'new-account' && accountsUrl}
	<NewAccountDialog bind:open url={accountsUrl} oncreated={created} />
{:else if shown?.dialog === 'reset-password' && accountsUrl}
	{@const account = shown.account}
	<ResetPasswordDialog
		bind:open
		url={accountsUrl}
		{account}
		onreset={() => toast.success(`Reset the password of ${account.username}`)}
	/>
{:else if shown?.dialog === 'delete-account' && accountsUrl}
	{@const account = shown.account}
	<ConfirmDialog
		bind:open
		title="Delete {account.username}?"
		action="Delete account"
		confirmName={account.username}
		onconfirm={() => deleteAccount(accountsUrl, account)}
	>
		This deletes the local account {account.username}, which no one has signed in with yet. It
		cannot be undone.
	</ConfirmDialog>
{:else if shown?.dialog === 'allowance' && directory.value}
	<AllowanceDialog bind:open user={shown.user} limit={directory.value.limit} onsaved={allowed} />
{:else if shown?.dialog === 'disable'}
	{@const user = shown.user}
	<ConfirmDialog
		bind:open
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
{:else if shown?.dialog === 'delete-user'}
	{@const user = shown.user}
	<ConfirmDialog
		bind:open
		title="Delete {user.handle}?"
		action="Delete user"
		confirmName={user.handle}
		onconfirm={() => deleteUser(user)}
	>
		This deletes {user.handle} with their sandboxes, SSH keys and environment. It cannot be undone.
	</ConfirmDialog>
{/if}
