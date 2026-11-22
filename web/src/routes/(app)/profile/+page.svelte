<script lang="ts">
	import { goto } from '$app/navigation';
	import { deleteAccount } from '#lib/api/me.js';
	import ConfirmDialog from '#lib/components/confirm-dialog.svelte';
	import PageHeader from '#lib/components/page-header.svelte';
	import UserAvatar from '#lib/components/user-avatar.svelte';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
	const user = $derived(data.user);

	const roles = { member: 'Member', admin: 'Admin' } as const;

	let deleting = $state(false);
	let typed = $state('');

	async function remove() {
		await deleteAccount();
		await goto('/sign-in', { replace: true, invalidateAll: true });
	}
</script>

<div class="grid grid-cols-1 gap-6">
	<PageHeader title="Profile" description="Your account." />

	<Card.Root>
		<Card.Content class="flex flex-wrap items-center gap-4">
			<UserAvatar handle={user.handle} picture={user.picture} class="size-16 text-xl" />
			<div class="grid min-w-0 gap-1">
				<div class="flex flex-wrap items-center gap-2">
					<h2 class="text-lg font-semibold break-all">{user.handle}</h2>
					<Badge variant={user.role === 'admin' ? 'default' : 'secondary'}>
						{roles[user.role]}
					</Badge>
				</div>
				<p class="text-sm break-all text-muted-foreground">{user.email ?? 'No email address'}</p>
			</div>
		</Card.Content>
	</Card.Root>

	<Card.Root class="ring-destructive/30">
		<Card.Header>
			<Card.Title>Delete your account</Card.Title>
			<Card.Description>
				Deletes your sandboxes with their home directories, your SSH keys, your credentials and your
				account, for good.
			</Card.Description>
		</Card.Header>
		<Card.Footer>
			<Button
				variant="destructive"
				onclick={() => {
					typed = '';
					deleting = true;
				}}
			>
				Delete account
			</Button>
		</Card.Footer>
	</Card.Root>
</div>

<ConfirmDialog
	bind:open={deleting}
	title="Delete your account?"
	action="Delete account"
	ready={typed === user.handle}
	onconfirm={remove}
>
	Every sandbox you own is deleted with its home directory, along with your SSH keys and
	credentials. It cannot be undone.
	{#snippet fields()}
		<div class="grid gap-2">
			<Label for="confirm-handle">
				Type <span class="font-mono">{user.handle}</span> to confirm
			</Label>
			<Input
				id="confirm-handle"
				bind:value={typed}
				autocomplete="off"
				autocapitalize="none"
				spellcheck={false}
			/>
		</div>
	{/snippet}
</ConfirmDialog>
