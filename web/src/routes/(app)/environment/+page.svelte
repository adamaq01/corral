<script lang="ts">
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import FileTextIcon from '@lucide/svelte/icons/file-text';
	import InfoIcon from '@lucide/svelte/icons/info';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import VariableIcon from '@lucide/svelte/icons/variable';
	import { goto } from '$app/navigation';
	import { environment } from '#lib/api/environment.js';
	import type { EntryKind, EnvironmentEntry } from '#lib/api/generated/index.js';
	import ListSkeleton from '#lib/components/list-skeleton.svelte';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import NewEntryDialog from '#lib/components/new-entry-dialog.svelte';
	import PageHeader from '#lib/components/page-header.svelte';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Empty from '#lib/components/ui/empty/index.js';
	import * as Item from '#lib/components/ui/item/index.js';
	import { summary, where } from '#lib/environment.js';
	import { Remote } from '#lib/remote.svelte.js';

	const list = new Remote(environment.list);

	let adding = $state(false);
	let kind = $state<EntryKind>('variable');

	function add(which: EntryKind) {
		kind = which;
		adding = true;
	}

	async function created(entry: EnvironmentEntry) {
		await goto(`/environment/${entry.id}`);
	}

	function attachedTo(entry: EnvironmentEntry): string {
		const count = entry.sandboxes.length;
		if (count === 0) {
			return 'Attached to no sandbox';
		}
		return `Attached to ${entry.sandboxes.join(', ')}`;
	}
</script>

{#snippet addButtons(primary: boolean)}
	<div class="flex flex-wrap gap-2">
		<Button variant={primary ? 'default' : 'outline'} onclick={() => add('variable')}>
			<PlusIcon />
			Add variable
		</Button>
		<Button variant="outline" onclick={() => add('file')}>
			<PlusIcon />
			Add file
		</Button>
	</div>
{/snippet}

<div class="grid grid-cols-1 gap-6">
	<PageHeader
		title="Environment"
		description="Variables and files your sandboxes receive, such as API keys and tool settings."
	>
		{#snippet actions()}
			{#if list.value?.length}
				{@render addButtons(true)}
			{/if}
		{/snippet}
	</PageHeader>

	<Alert.Root>
		<InfoIcon />
		<Alert.Title>Changes reach a sandbox when it next starts</Alert.Title>
		<Alert.Description>
			A sandbox gets the entries attached to it, which you choose on its page. A sleeping sandbox
			gets the current values when it wakes. A running one keeps what it started with until it
			restarts, and Sandboxes marks it until then.
		</Alert.Description>
	</Alert.Root>

	{#if list.current.state === 'loading'}
		<ListSkeleton what="your environment" />
	{:else if list.current.state === 'failed'}
		<LoadFailed
			title="Loading your environment failed"
			error={list.current.error}
			onretry={() => list.reload()}
		/>
	{:else if list.current.value.length === 0}
		<Empty.Root class="border">
			<Empty.Header>
				<Empty.Media variant="icon"><VariableIcon /></Empty.Media>
				<Empty.Title>No variables or files yet</Empty.Title>
				<Empty.Description>
					Store an API key, a git token or a config file once, and attach it to any sandbox you own.
				</Empty.Description>
			</Empty.Header>
			<Empty.Content>
				{@render addButtons(true)}
			</Empty.Content>
		</Empty.Root>
	{:else}
		<ul class="grid gap-2" aria-label="Environment">
			{#each list.current.value as entry (entry.id)}
				{@const Icon = entry.kind === 'variable' ? VariableIcon : FileTextIcon}
				<li>
					<Item.Root variant="outline">
						{#snippet child({ props })}
							<a href="/environment/{entry.id}" {...props}>
								<Item.Media variant="icon">
									<Icon class="text-muted-foreground" />
								</Item.Media>
								<Item.Content class="min-w-0">
									<Item.Title class="max-w-full font-mono break-all">
										{where(entry)}
										{#if entry.secret}
											<Badge variant="secondary">Secret</Badge>
										{/if}
										{#if entry.template}
											<Badge variant="outline">Template</Badge>
										{/if}
									</Item.Title>
									<Item.Description class="break-all">
										{entry.kind === 'variable' || entry.value.shown !== 'text'
											? summary(entry.value)
											: `${entry.value.text.split('\n').length} lines`} · {attachedTo(
											entry
										)}{entry.applyToNew ? ' · New sandboxes get it' : ''}
									</Item.Description>
								</Item.Content>
								<Item.Actions>
									<ChevronRightIcon class="size-4 text-muted-foreground" aria-hidden="true" />
								</Item.Actions>
							</a>
						{/snippet}
					</Item.Root>
				</li>
			{/each}
		</ul>
	{/if}
</div>

<NewEntryDialog bind:open={adding} {kind} onCreated={created} />
