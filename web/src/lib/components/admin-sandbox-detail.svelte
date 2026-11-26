<script lang="ts">
	import ChevronLeftIcon from '@lucide/svelte/icons/chevron-left';
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import SearchXIcon from '@lucide/svelte/icons/search-x';
	import { sandboxesOf } from '#lib/all-sandboxes.js';
	import { sandboxDetails } from '#lib/api/admin.js';
	import { sandboxes } from '#lib/api/sandboxes.js';
	import { choiceLabel } from '#lib/catalog.js';
	import EngineEvents from '#lib/components/engine-events.svelte';
	import EngineFacts from '#lib/components/engine-facts.svelte';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import RefreshStatus from '#lib/components/refresh-status.svelte';
	import SandboxStateBadge from '#lib/components/sandbox-state-badge.svelte';
	import Timestamp from '#lib/components/timestamp.svelte';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Empty from '#lib/components/ui/empty/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { Spinner } from '#lib/components/ui/spinner/index.js';
	import UserAvatar from '#lib/components/user-avatar.svelte';
	import { Remote, keepFresh } from '#lib/remote.svelte.js';
	import { states } from '#lib/sandbox-status.js';

	let { handle, name }: { handle: string; name: string } = $props();

	const details = new Remote(() => sandboxDetails(handle, name));
	const catalog = new Remote(sandboxes.catalog);
	let refreshing = $state(false);

	keepFresh(details, () => (details.value ? states[details.value.sandbox.state].settling : false));

	async function refresh() {
		refreshing = true;
		await details.reload();
		refreshing = false;
	}
</script>

<svelte:head>
	<title>{name}.{handle} · Corral</title>
</svelte:head>

<div class="grid grid-cols-1 gap-6">
	<a
		href="/admin/sandboxes"
		class="-ml-1 inline-flex w-fit items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
	>
		<ChevronLeftIcon class="size-4" />
		All sandboxes
	</a>

	<header class="flex flex-wrap items-start justify-between gap-x-6 gap-y-4">
		<div class="grid min-w-0 gap-1">
			<div class="flex flex-wrap items-center gap-2">
				<h1 class="text-2xl font-semibold tracking-tight break-all">{name}</h1>
				{#if details.value}
					<SandboxStateBadge status={details.value.sandbox} />
					{#if details.value.sandbox.isDefault}
						<Badge variant="secondary">Default</Badge>
					{/if}
					{#if details.value.sandbox.deleteAt}
						<Badge variant="destructive">Deletion scheduled</Badge>
					{/if}
					{#if details.value.sandbox.staleEnvironment}
						<Badge variant="outline">Restart pending</Badge>
					{/if}
				{/if}
			</div>
			<p class="flex items-center gap-2 text-sm">
				<UserAvatar
					{handle}
					picture={details.value?.sandbox.owner.picture ?? null}
					size="sm"
					class="shrink-0"
				/>
				<a
					href={sandboxesOf(handle)}
					class="rounded-sm font-medium break-all underline decoration-muted-foreground/50 underline-offset-4 hover:decoration-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none"
					title="Sandboxes of {handle}"
				>
					{handle}
				</a>
			</p>
			{#if details.value}
				{@const { sandbox } = details.value}
				<p class="text-sm text-muted-foreground">
					{choiceLabel(catalog.value?.images, sandbox.image)} · {choiceLabel(
						catalog.value?.sizes,
						sandbox.size
					)} · Created <Timestamp value={sandbox.createdAt} />
				</p>
				<p class="text-sm text-muted-foreground">
					SSH username
					<code class="font-mono text-xs wrap-anywhere text-foreground">
						{sandbox.ssh.username}
					</code>
				</p>
			{/if}
		</div>
		{#if details.value}
			<div class="flex flex-wrap items-center gap-x-3 gap-y-2">
				<RefreshStatus remote={details} time />
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
		{/if}
	</header>

	{#if details.current.state === 'loading'}
		<div class="grid gap-4" role="status" aria-label="Loading the sandbox">
			<Skeleton class="h-96 rounded-xl" />
			<Skeleton class="h-40 rounded-xl" />
		</div>
	{:else if details.current.state === 'failed' && details.current.status === 404}
		<Empty.Root class="border">
			<Empty.Header>
				<Empty.Media variant="icon"><SearchXIcon /></Empty.Media>
				<Empty.Title>{handle} has no sandbox named {name}</Empty.Title>
				<Empty.Description>It may have been deleted.</Empty.Description>
			</Empty.Header>
			<Empty.Content>
				<Button href="/admin/sandboxes" variant="outline">See all sandboxes</Button>
			</Empty.Content>
		</Empty.Root>
	{:else if details.current.state === 'failed'}
		<LoadFailed
			title="Loading the sandbox failed"
			error={details.current.error}
			onretry={() => details.reload()}
		/>
	{:else}
		{@const { sandbox, facts, events } = details.current.value}
		{#if sandbox.state === 'failed'}
			<Alert.Root variant="destructive">
				<CircleAlertIcon />
				<Alert.Title>The sandbox failed</Alert.Title>
				<Alert.Description class="break-words first-letter:uppercase">
					{sandbox.detail}
				</Alert.Description>
			</Alert.Root>
		{/if}

		<Card.Root>
			<Card.Header>
				<Card.Title><h2>What it runs on</h2></Card.Title>
				<Card.Description>
					As the engine reports it now. None marks what does not exist at the moment, such as the
					pod of a sleeping sandbox.
				</Card.Description>
			</Card.Header>
			<Card.Content>
				<EngineFacts {facts} />
			</Card.Content>
		</Card.Root>

		<Card.Root>
			<Card.Header>
				<Card.Title><h2>Recent events</h2></Card.Title>
				<Card.Description>The most recent first.</Card.Description>
			</Card.Header>
			<Card.Content>
				<EngineEvents {events} />
			</Card.Content>
		</Card.Root>
	{/if}
</div>
