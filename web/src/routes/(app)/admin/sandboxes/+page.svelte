<script lang="ts">
	import BoxesIcon from '@lucide/svelte/icons/boxes';
	import { allSandboxes } from '#lib/api/admin.js';
	import { sandboxes as sandboxApi } from '#lib/api/sandboxes.js';
	import { choiceLabel } from '#lib/catalog.js';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import PageHeader from '#lib/components/page-header.svelte';
	import SandboxStateBadge from '#lib/components/sandbox-state-badge.svelte';
	import TableSkeleton from '#lib/components/table-skeleton.svelte';
	import Timestamp from '#lib/components/timestamp.svelte';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import * as Empty from '#lib/components/ui/empty/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import UserAvatar from '#lib/components/user-avatar.svelte';
	import { Remote } from '#lib/remote.svelte.js';

	const wide = 'hidden sm:table-cell';
	const columns = [wide, '', '', wide, wide, wide];

	const listing = new Remote(async () => {
		const [sandboxes, catalog] = await Promise.all([allSandboxes(), sandboxApi.catalog()]);
		return { sandboxes, catalog };
	});
</script>

<div class="space-y-6">
	<PageHeader title="All sandboxes" description="Every user's sandboxes." />

	{#if listing.current.state === 'failed'}
		<LoadFailed
			title="Loading the sandboxes failed"
			error={listing.current.error}
			onretry={() => listing.reload()}
		/>
	{:else if listing.current.state === 'ready' && listing.current.value.sandboxes.length === 0}
		<Empty.Root class="border">
			<Empty.Header>
				<Empty.Media variant="icon"><BoxesIcon /></Empty.Media>
				<Empty.Title>No sandboxes yet</Empty.Title>
				<Empty.Description>Sandboxes appear here as soon as anyone creates one.</Empty.Description>
			</Empty.Header>
		</Empty.Root>
	{:else}
		<div class="rounded-lg border">
			<Table.Root aria-label="All sandboxes" aria-busy={listing.current.state === 'loading'}>
				<Table.Header>
					<Table.Row>
						<Table.Head class={wide}>Owner</Table.Head>
						<Table.Head>
							<span class="sm:hidden">Sandbox</span><span class="hidden sm:inline">Name</span>
						</Table.Head>
						<Table.Head>Status</Table.Head>
						<Table.Head class={wide}>Image</Table.Head>
						<Table.Head class={wide}>Size</Table.Head>
						<Table.Head class={wide}>Created</Table.Head>
					</Table.Row>
				</Table.Header>
				<Table.Body>
					{#if listing.current.state === 'loading'}
						<TableSkeleton {columns} />
					{:else}
						{@const { sandboxes, catalog } = listing.current.value}
						{#each sandboxes as sandbox (`${sandbox.owner.handle}+${sandbox.name}`)}
							{@const image = choiceLabel(catalog.images, sandbox.image)}
							{@const size = choiceLabel(catalog.sizes, sandbox.size)}
							<Table.Row>
								<Table.Cell class={wide}>
									<div class="flex items-center gap-2">
										<UserAvatar
											handle={sandbox.owner.handle}
											picture={sandbox.owner.picture}
											size="sm"
										/>
										<span class="font-medium">{sandbox.owner.handle}</span>
									</div>
								</Table.Cell>
								<Table.Cell>
									<div class="grid gap-0.5">
										<span class="flex items-center gap-2 font-medium sm:font-normal">
											{sandbox.name}
											{#if sandbox.isDefault}
												<Badge variant="secondary">Default</Badge>
											{/if}
										</span>
										<span class="text-xs text-muted-foreground sm:hidden">
											{sandbox.owner.handle}
										</span>
										<span class="text-xs text-muted-foreground sm:hidden">{image} · {size}</span>
									</div>
								</Table.Cell>
								<Table.Cell><SandboxStateBadge status={sandbox} detail /></Table.Cell>
								<Table.Cell class={wide}>{image}</Table.Cell>
								<Table.Cell class={wide}>{size}</Table.Cell>
								<Table.Cell class={wide}><Timestamp value={sandbox.createdAt} /></Table.Cell>
							</Table.Row>
						{/each}
					{/if}
				</Table.Body>
			</Table.Root>
		</div>
	{/if}
</div>
