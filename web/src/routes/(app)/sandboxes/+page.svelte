<script lang="ts">
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import SquareTerminalIcon from '@lucide/svelte/icons/square-terminal';
	import { goto } from '$app/navigation';
	import type { Sandbox } from '#lib/api/generated/index.js';
	import { sandboxes } from '#lib/api/sandboxes.js';
	import CreateSandboxDialog from '#lib/components/create-sandbox-dialog.svelte';
	import ListSkeleton from '#lib/components/list-skeleton.svelte';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import PageHeader from '#lib/components/page-header.svelte';
	import SandboxStateBadge from '#lib/components/sandbox-state-badge.svelte';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Empty from '#lib/components/ui/empty/index.js';
	import * as Item from '#lib/components/ui/item/index.js';
	import { choiceLabel } from '#lib/catalog.js';
	import { Remote, pollWhile } from '#lib/remote.svelte.js';
	import { states } from '#lib/sandbox-status.js';

	const list = new Remote(sandboxes.list);
	const catalog = new Remote(sandboxes.catalog);
	pollWhile(() => list.value?.some((sandbox) => states[sandbox.state].settling) ?? false, list);

	let creating = $state(false);

	async function created(sandbox: Sandbox) {
		await goto(`/sandboxes/${sandbox.name}`);
	}
</script>

{#snippet newSandbox()}
	<Button onclick={() => (creating = true)} disabled={!catalog.value || !list.value}>
		<PlusIcon />
		New sandbox
	</Button>
{/snippet}

<div class="grid grid-cols-1 gap-6">
	<PageHeader title="Sandboxes" description="Your persistent machines, reachable over SSH.">
		{#snippet actions()}
			{#if list.value?.length}
				{@render newSandbox()}
			{/if}
		{/snippet}
	</PageHeader>

	{#if catalog.current.state === 'failed'}
		<LoadFailed
			title="Loading the catalog failed"
			error={catalog.current.error}
			onretry={() => catalog.reload()}
		/>
	{/if}

	{#if list.current.state === 'loading'}
		<ListSkeleton what="sandboxes" />
	{:else if list.current.state === 'failed'}
		<LoadFailed
			title="Loading your sandboxes failed"
			error={list.current.error}
			onretry={() => list.reload()}
		/>
	{:else if list.current.value.length === 0}
		<Empty.Root class="border">
			<Empty.Header>
				<Empty.Media variant="icon"><SquareTerminalIcon /></Empty.Media>
				<Empty.Title>No sandboxes yet</Empty.Title>
				<Empty.Description>
					Create one, then connect to it with SSH or Herdr using a key from
					<a href="/keys">SSH keys</a>.
				</Empty.Description>
			</Empty.Header>
			<Empty.Content>
				{@render newSandbox()}
			</Empty.Content>
		</Empty.Root>
	{:else}
		<ul class="grid gap-2" aria-label="Sandboxes">
			{#each list.current.value as sandbox (sandbox.name)}
				<li>
					<Item.Root variant="outline">
						{#snippet child({ props })}
							<a href="/sandboxes/{sandbox.name}" {...props}>
								<Item.Content class="min-w-0">
									<Item.Title>
										{sandbox.name}
										{#if sandbox.isDefault}
											<Badge variant="secondary">Default</Badge>
										{/if}
									</Item.Title>
									<Item.Description>
										{choiceLabel(catalog.value?.images, sandbox.image)} · {choiceLabel(
											catalog.value?.sizes,
											sandbox.size
										)}
									</Item.Description>
									{#if sandbox.state === 'failed'}
										<p class="text-sm break-words text-destructive first-letter:uppercase">
											{sandbox.detail}
										</p>
									{/if}
								</Item.Content>
								<Item.Actions>
									<SandboxStateBadge status={sandbox} />
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

{#if catalog.value && list.value}
	<CreateSandboxDialog
		bind:open={creating}
		catalog={catalog.value}
		count={list.value.length}
		onCreated={created}
	/>
{/if}
