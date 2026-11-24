<script lang="ts">
	import BoxesIcon from '@lucide/svelte/icons/boxes';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { allSandboxes } from '#lib/api/admin.js';
	import { sandboxes as sandboxApi } from '#lib/api/sandboxes.js';
	import { choiceLabel } from '#lib/catalog.js';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import PageHeader from '#lib/components/page-header.svelte';
	import RefreshStatus from '#lib/components/refresh-status.svelte';
	import SandboxStateBadge from '#lib/components/sandbox-state-badge.svelte';
	import StateAnnouncer from '#lib/components/state-announcer.svelte';
	import TableSkeleton from '#lib/components/table-skeleton.svelte';
	import Timestamp from '#lib/components/timestamp.svelte';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Empty from '#lib/components/ui/empty/index.js';
	import * as Field from '#lib/components/ui/field/index.js';
	import * as NativeSelect from '#lib/components/ui/native-select/index.js';
	import { Spinner } from '#lib/components/ui/spinner/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import UserAvatar from '#lib/components/user-avatar.svelte';
	import { Remote, keepFresh } from '#lib/remote.svelte.js';
	import { states } from '#lib/sandbox-status.js';

	const wide = 'hidden sm:table-cell';
	const columns = [wide, '', '', wide, wide, wide];

	const listing = new Remote(async () => {
		const [sandboxes, catalog] = await Promise.all([allSandboxes(), sandboxApi.catalog()]);
		return { sandboxes, catalog };
	});
	keepFresh(
		listing,
		() => listing.value?.sandboxes.some((sandbox) => states[sandbox.state].settling) ?? false
	);

	let refreshing = $state(false);

	async function refresh() {
		refreshing = true;
		await listing.reload();
		refreshing = false;
	}

	const owner = $derived(page.url.searchParams.get('owner') ?? '');
	const owners = $derived(
		[...new Set(listing.value?.sandboxes.map((sandbox) => sandbox.owner.handle))].sort()
	);
	const shown = $derived(
		listing.value?.sandboxes.filter((sandbox) => owner === '' || sandbox.owner.handle === owner)
	);

	function filter(handle: string) {
		const url = new URL(page.url.href);
		if (handle) {
			url.searchParams.set('owner', handle);
		} else {
			url.searchParams.delete('owner');
		}
		void goto(url, { replace: true, reset: false });
	}
</script>

<div class="space-y-6">
	<PageHeader title="All sandboxes" description="Every user's sandboxes.">
		{#snippet actions()}
			<div class="flex flex-wrap items-center gap-x-3 gap-y-2">
				<RefreshStatus remote={listing} time />
				<Button variant="outline" disabled={refreshing} onclick={refresh}>
					{#if refreshing}
						<Spinner role="presentation" aria-label={undefined} aria-hidden="true" />
						Refreshing…
					{:else}
						<RefreshCwIcon />
						Refresh
					{/if}
				</Button>
			</div>
		{/snippet}
	</PageHeader>

	<StateAnnouncer sandboxes={listing.value?.sandboxes} />

	{#if listing.value && (owners.length > 1 || owner !== '')}
		<Field.Field orientation="horizontal" class="w-fit">
			<Field.Label for="owner-filter" class="shrink-0">Owner</Field.Label>
			<NativeSelect.Root
				id="owner-filter"
				value={owner}
				onchange={(event) => filter(event.currentTarget.value)}
			>
				<NativeSelect.Option value="">Everyone</NativeSelect.Option>
				{#each owners as handle (handle)}
					<NativeSelect.Option value={handle}>{handle}</NativeSelect.Option>
				{/each}
				{#if owner !== '' && !owners.includes(owner)}
					<NativeSelect.Option value={owner}>{owner}</NativeSelect.Option>
				{/if}
			</NativeSelect.Root>
		</Field.Field>
	{/if}

	{#if listing.current.state === 'failed'}
		<LoadFailed
			title="Loading the sandboxes failed"
			error={listing.current.error}
			onretry={() => listing.reload()}
		/>
	{:else if shown?.length === 0}
		<Empty.Root class="border">
			<Empty.Header>
				<Empty.Media variant="icon"><BoxesIcon /></Empty.Media>
				{#if owner}
					<Empty.Title>{owner} has no sandboxes</Empty.Title>
					<Empty.Description>
						Their sandboxes appear here as soon as they create one.
					</Empty.Description>
				{:else}
					<Empty.Title>No sandboxes yet</Empty.Title>
					<Empty.Description>
						Sandboxes appear here as soon as anyone creates one.
					</Empty.Description>
				{/if}
			</Empty.Header>
			{#if owner}
				<Empty.Content>
					<Button variant="outline" onclick={() => filter('')}>Show everyone's</Button>
				</Empty.Content>
			{/if}
		</Empty.Root>
	{:else}
		<div class="rounded-lg border">
			<Table.Root aria-label="All sandboxes" aria-busy={listing.current.state === 'loading'}>
				<Table.Header>
					<Table.Row>
						<Table.Head class={wide}>Owner</Table.Head>
						<Table.Head>Sandbox</Table.Head>
						<Table.Head>Status</Table.Head>
						<Table.Head class={wide}>Image</Table.Head>
						<Table.Head class={wide}>Size</Table.Head>
						<Table.Head class={wide}>Created</Table.Head>
					</Table.Row>
				</Table.Header>
				<Table.Body>
					{#if !listing.value || !shown}
						<TableSkeleton {columns} />
					{:else}
						{@const catalog = listing.value.catalog}
						{#each shown as sandbox (sandbox.ssh.username)}
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
									<div class="grid justify-items-start gap-1">
										<span class="flex flex-wrap items-center gap-2 font-medium sm:font-normal">
											<a
												href="/admin/sandboxes/{sandbox.owner.handle}/{sandbox.name}"
												class="rounded-sm underline decoration-muted-foreground/50 underline-offset-4 hover:decoration-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none"
												aria-label="{sandbox.name}.{sandbox.owner.handle} details"
											>
												{sandbox.name}
											</a>
											{#if sandbox.isDefault}
												<Badge variant="secondary">Default</Badge>
											{/if}
										</span>
										{#if sandbox.deleteAt || sandbox.staleEnvironment}
											<span class="flex flex-wrap gap-1">
												{#if sandbox.deleteAt}
													<Badge variant="destructive">Deletion scheduled</Badge>
												{/if}
												{#if sandbox.staleEnvironment}
													<Badge variant="outline">Restart pending</Badge>
												{/if}
											</span>
										{/if}
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
