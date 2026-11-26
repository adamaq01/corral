<script lang="ts">
	import KeyRoundIcon from '@lucide/svelte/icons/key-round';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import { toast } from 'svelte-sonner';
	import type { OpksshLogin, SshKey } from '#lib/api/generated/index.js';
	import { keys } from '#lib/api/keys.js';
	import AddKeyDialog from '#lib/components/add-key-dialog.svelte';
	import ConfirmDialog from '#lib/components/confirm-dialog.svelte';
	import CopyButton from '#lib/components/copy-button.svelte';
	import ListSkeleton from '#lib/components/list-skeleton.svelte';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import Timestamp from '#lib/components/timestamp.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Empty from '#lib/components/ui/empty/index.js';
	import * as Item from '#lib/components/ui/item/index.js';
	import { Remote } from '#lib/remote.svelte.js';

	let { opkssh }: { opkssh: OpksshLogin | null } = $props();

	const list = new Remote(keys.list);

	let adding = $state(false);
	let removing = $state<SshKey | null>(null);
	let confirming = $state(false);

	function askToRemove(key: SshKey) {
		removing = key;
		confirming = true;
	}

	async function remove() {
		if (removing) {
			await keys.remove(removing.id);
			toast.success(`Deleted the key ${removing.label}`);
			await list.reload();
		}
	}

	async function added(label: string) {
		toast.success(`Added the key ${label}`);
		await list.reload();
	}
</script>

{#snippet addKey()}
	<Button onclick={() => (adding = true)}>
		<PlusIcon />
		Add key
	</Button>
{/snippet}

<Card.Root id="keys" class="scroll-mt-20">
	<Card.Header>
		<Card.Title><h2>SSH keys</h2></Card.Title>
		<Card.Description>The public keys that may open your sandboxes.</Card.Description>
		{#if list.value?.length}
			<Card.Action>{@render addKey()}</Card.Action>
		{/if}
	</Card.Header>
	<Card.Content class="grid gap-4">
		{#if list.current.state === 'loading'}
			<ListSkeleton what="SSH keys" />
		{:else if list.current.state === 'failed'}
			<LoadFailed
				title="Loading your SSH keys failed"
				error={list.current.error}
				onretry={() => list.reload()}
			/>
		{:else if list.current.value.length === 0}
			<Empty.Root class="border">
				<Empty.Header>
					<Empty.Media variant="icon"><KeyRoundIcon /></Empty.Media>
					<Empty.Title>No SSH keys yet</Empty.Title>
					<Empty.Description>
						Add the public key of each computer you connect from. Corral never asks for a password.
						{#if opkssh}
							With opkssh, your single sign-on login can stand in for a key: each sandbox's page
							shows how.
						{/if}
					</Empty.Description>
				</Empty.Header>
				<Empty.Content>
					{@render addKey()}
				</Empty.Content>
			</Empty.Root>
		{:else}
			{#if opkssh}
				<p class="text-sm text-muted-foreground">
					With opkssh, your single sign-on login can stand in for a key: each sandbox's page shows
					how.
				</p>
			{/if}
			<ul class="grid gap-2" aria-label="SSH keys">
				{#each list.current.value as key (key.id)}
					<li>
						<Item.Root variant="outline">
							<Item.Media variant="icon"><KeyRoundIcon class="text-muted-foreground" /></Item.Media>
							<Item.Content class="min-w-0">
								<Item.Title class="line-clamp-none wrap-anywhere">{key.label}</Item.Title>
								<Item.Description class="font-mono text-xs break-all">
									{key.fingerprint}
								</Item.Description>
								<Item.Description class="text-xs">
									Added <Timestamp value={key.createdAt} />
								</Item.Description>
							</Item.Content>
							<Item.Actions>
								<CopyButton text={key.publicKey} label="the public key {key.label}" />
								<Button
									variant="ghost"
									size="icon-sm"
									aria-label="Delete key {key.label}"
									title="Delete"
									onclick={() => askToRemove(key)}
								>
									<Trash2Icon />
								</Button>
							</Item.Actions>
						</Item.Root>
					</li>
				{/each}
			</ul>
		{/if}
	</Card.Content>
</Card.Root>

<AddKeyDialog bind:open={adding} onAdded={added} />

<ConfirmDialog
	bind:open={confirming}
	title="Delete key {removing?.label}?"
	action="Delete key"
	onconfirm={remove}
>
	<p>New SSH connections with this key are refused from now on.</p>
	{#if list.value?.length === 1}
		<p class="font-medium text-destructive">
			It is your last key. Without one, you cannot connect to your sandboxes over SSH{opkssh
				? ' unless you use opkssh'
				: ''}.
		</p>
	{/if}
	<p class="font-mono text-xs break-all">{removing?.fingerprint}</p>
</ConfirmDialog>
