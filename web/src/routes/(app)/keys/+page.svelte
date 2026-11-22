<script lang="ts">
	import KeyRoundIcon from '@lucide/svelte/icons/key-round';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import type { SshKey } from '#lib/api/generated/index.js';
	import { keys } from '#lib/api/keys.js';
	import AddKeyDialog from '#lib/components/add-key-dialog.svelte';
	import ConfirmDialog from '#lib/components/confirm-dialog.svelte';
	import ListSkeleton from '#lib/components/list-skeleton.svelte';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import PageHeader from '#lib/components/page-header.svelte';
	import Timestamp from '#lib/components/timestamp.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Empty from '#lib/components/ui/empty/index.js';
	import * as Item from '#lib/components/ui/item/index.js';
	import { Remote } from '#lib/remote.svelte.js';

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
			await list.reload();
		}
	}
</script>

{#snippet addKey()}
	<Button onclick={() => (adding = true)}>
		<PlusIcon />
		Add key
	</Button>
{/snippet}

<div class="grid grid-cols-1 gap-6">
	<div class="flex flex-wrap items-end justify-between gap-4">
		<PageHeader title="SSH keys" description="The public keys that may open your sandboxes." />
		{#if list.value?.length}
			{@render addKey()}
		{/if}
	</div>

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
				</Empty.Description>
			</Empty.Header>
			<Empty.Content>
				{@render addKey()}
			</Empty.Content>
		</Empty.Root>
	{:else}
		<ul class="grid gap-2" aria-label="SSH keys">
			{#each list.current.value as key (key.id)}
				<li>
					<Item.Root variant="outline">
						<Item.Media variant="icon"><KeyRoundIcon class="text-muted-foreground" /></Item.Media>
						<Item.Content class="min-w-0">
							<Item.Title class="max-w-full truncate">{key.label}</Item.Title>
							<Item.Description class="font-mono text-xs break-all">
								{key.fingerprint}
							</Item.Description>
							<Item.Description class="text-xs">
								Added <Timestamp value={key.createdAt} />
							</Item.Description>
						</Item.Content>
						<Item.Actions>
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
</div>

<AddKeyDialog bind:open={adding} onAdded={() => list.reload()} />

<ConfirmDialog
	bind:open={confirming}
	title="Delete key {removing?.label}?"
	action="Delete key"
	onconfirm={remove}
>
	<p>New SSH connections with this key are refused from now on.</p>
	<p class="font-mono text-xs break-all">{removing?.fingerprint}</p>
</ConfirmDialog>
