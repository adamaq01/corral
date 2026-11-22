<script lang="ts">
	import InfoIcon from '@lucide/svelte/icons/info';
	import LockKeyholeIcon from '@lucide/svelte/icons/lock-keyhole';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import { credentials } from '#lib/api/credentials.js';
	import ConfirmDialog from '#lib/components/confirm-dialog.svelte';
	import ListSkeleton from '#lib/components/list-skeleton.svelte';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import PageHeader from '#lib/components/page-header.svelte';
	import SetCredentialDialog from '#lib/components/set-credential-dialog.svelte';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Empty from '#lib/components/ui/empty/index.js';
	import * as Item from '#lib/components/ui/item/index.js';
	import { Remote } from '#lib/remote.svelte.js';

	const list = new Remote(credentials.list);
	const names = $derived(list.value?.map((credential) => credential.name) ?? []);

	let setting = $state(false);
	let replacing = $state<string | null>(null);
	let removing = $state<string | null>(null);
	let confirming = $state(false);

	function openSet(name: string | null) {
		replacing = name;
		setting = true;
	}

	function askToRemove(name: string) {
		removing = name;
		confirming = true;
	}

	async function remove() {
		if (removing) {
			await credentials.remove(removing);
			await list.reload();
		}
	}
</script>

{#snippet addCredential()}
	<Button onclick={() => openSet(null)}>
		<PlusIcon />
		Add credential
	</Button>
{/snippet}

<div class="grid grid-cols-1 gap-6">
	<PageHeader
		title="Credentials"
		description="Secrets your sandboxes receive, such as LLM API keys. Values are never shown again."
	>
		{#snippet actions()}
			{#if list.value?.length}
				{@render addCredential()}
			{/if}
		{/snippet}
	</PageHeader>

	<Alert.Root>
		<InfoIcon />
		<Alert.Title>Values reach a sandbox when it next starts</Alert.Title>
		<Alert.Description>
			Each credential is an environment variable in every SSH session. A sleeping sandbox gets the
			current values when it wakes; restart a running one to pick up a change.
		</Alert.Description>
	</Alert.Root>

	{#if list.current.state === 'loading'}
		<ListSkeleton what="credentials" />
	{:else if list.current.state === 'failed'}
		<LoadFailed
			title="Loading your credentials failed"
			error={list.current.error}
			onretry={() => list.reload()}
		/>
	{:else if list.current.value.length === 0}
		<Empty.Root class="border">
			<Empty.Header>
				<Empty.Media variant="icon"><LockKeyholeIcon /></Empty.Media>
				<Empty.Title>No credentials yet</Empty.Title>
				<Empty.Description>
					Store an API key, a gateway URL or a git token once, and every sandbox you own gets it.
				</Empty.Description>
			</Empty.Header>
			<Empty.Content>
				{@render addCredential()}
			</Empty.Content>
		</Empty.Root>
	{:else}
		<ul class="grid gap-2" aria-label="Credentials">
			{#each list.current.value as credential (credential.name)}
				<li>
					<Item.Root variant="outline">
						<Item.Media variant="icon">
							<LockKeyholeIcon class="text-muted-foreground" />
						</Item.Media>
						<Item.Content class="min-w-0">
							<Item.Title class="max-w-full font-mono break-all">{credential.name}</Item.Title>
						</Item.Content>
						<Item.Actions>
							<Button variant="outline" size="sm" onclick={() => openSet(credential.name)}>
								Replace
							</Button>
							<Button
								variant="ghost"
								size="icon-sm"
								aria-label="Delete {credential.name}"
								title="Delete"
								onclick={() => askToRemove(credential.name)}
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

<SetCredentialDialog
	bind:open={setting}
	{replacing}
	existing={names}
	onSaved={() => list.reload()}
/>

<ConfirmDialog
	bind:open={confirming}
	title="Delete {removing}?"
	action="Delete credential"
	onconfirm={remove}
>
	<p>Its value is gone for good. Sandboxes lose it when they next start.</p>
</ConfirmDialog>
