<script lang="ts">
	import BoxesIcon from '@lucide/svelte/icons/boxes';
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import EllipsisIcon from '@lucide/svelte/icons/ellipsis';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import { SvelteMap } from 'svelte/reactivity';
	import { toast } from 'svelte-sonner';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { ownerOf, sandboxesOf, shown, type Row } from '#lib/all-sandboxes.js';
	import * as admin from '#lib/api/admin.js';
	import { failureMessage } from '#lib/api/client.js';
	import type { Handle } from '#lib/api/generated/index.js';
	import { sandboxes as sandboxApi } from '#lib/api/sandboxes.js';
	import { choiceLabel } from '#lib/catalog.js';
	import ConfirmDialog from '#lib/components/confirm-dialog.svelte';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import PageHeader from '#lib/components/page-header.svelte';
	import RefreshStatus from '#lib/components/refresh-status.svelte';
	import SandboxStateBadge from '#lib/components/sandbox-state-badge.svelte';
	import StateAnnouncer from '#lib/components/state-announcer.svelte';
	import TableSkeleton from '#lib/components/table-skeleton.svelte';
	import Timestamp from '#lib/components/timestamp.svelte';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import * as Empty from '#lib/components/ui/empty/index.js';
	import * as Field from '#lib/components/ui/field/index.js';
	import * as NativeSelect from '#lib/components/ui/native-select/index.js';
	import { Spinner } from '#lib/components/ui/spinner/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import UserAvatar from '#lib/components/user-avatar.svelte';
	import { Remote, keepFresh } from '#lib/remote.svelte.js';
	import { states, type Power } from '#lib/sandbox-status.js';

	const wide = 'hidden sm:table-cell';
	const columns = [wide, '', '', wide, wide, wide, ''];

	const listing = new Remote(async () => {
		const [sandboxes, catalog] = await Promise.all([admin.allSandboxes(), sandboxApi.catalog()]);
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

	const owner = $derived(ownerOf(page.url));
	const view = $derived(shown(listing.value?.sandboxes, owner));

	function filter(handle: Handle | null) {
		void goto(sandboxesOf(handle), { replace: true, reset: false });
	}

	const powers: Record<
		Power,
		{
			label: string;
			pending: string;
			run: (handle: Handle, name: string) => Promise<unknown>;
			done: (fullName: string) => string;
		}
	> = {
		wake: {
			label: 'Wake',
			pending: 'Waking…',
			run: admin.wakeSandbox,
			done: (fullName) => `Woke ${fullName}`
		},
		stop: {
			label: 'Sleep',
			pending: 'Going to sleep…',
			run: admin.stopSandbox,
			done: (fullName) => `Put ${fullName} to sleep`
		},
		restart: {
			label: 'Restart',
			pending: 'Restarting…',
			run: admin.restartSandbox,
			done: (fullName) => `Restarted ${fullName}`
		}
	};

	/** The powers in flight, by the full name of their sandbox. */
	const pending = new SvelteMap<string, Power>();
	let failure = $state<{ title: string; message: string } | null>(null);

	async function act({ sandbox, fullName }: Row, power: Power) {
		failure = null;
		pending.set(fullName, power);
		try {
			await powers[power].run(sandbox.owner.handle, sandbox.name);
			toast.success(powers[power].done(fullName));
		} catch (error) {
			failure = {
				title: `${powers[power].label} failed for ${fullName}`,
				message: failureMessage(error)
			};
		}
		await listing.reload();
		pending.delete(fullName);
	}

	/** The row whose deletion is asked, kept while the dialog closes. */
	let doomed = $state<Row | null>(null);
	let confirming = $state(false);

	function confirmDelete(row: Row) {
		doomed = row;
		confirming = true;
	}

	async function remove({ sandbox, fullName }: Row) {
		try {
			await admin.deleteSandbox(sandbox.owner.handle, sandbox.name);
			toast.success(`Deleted ${fullName}`);
		} finally {
			await listing.reload();
		}
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

	<Field.Field orientation="horizontal" class="w-fit">
		<Field.Label for="owner-filter" class="shrink-0">Owner</Field.Label>
		<NativeSelect.Root
			id="owner-filter"
			value={owner ?? ''}
			onchange={(event) => filter(event.currentTarget.value || null)}
		>
			<NativeSelect.Option value="">Everyone</NativeSelect.Option>
			{#each view.owners as handle (handle)}
				<NativeSelect.Option value={handle}>{handle}</NativeSelect.Option>
			{/each}
		</NativeSelect.Root>
	</Field.Field>

	{#if listing.current.state === 'failed'}
		<LoadFailed
			title="Loading the sandboxes failed"
			error={listing.current.error}
			onretry={() => listing.reload()}
		/>
	{:else if view.rows?.length === 0}
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
					<Button variant="outline" onclick={() => filter(null)}>Show everyone's</Button>
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
						<Table.Head><span class="sr-only">Actions</span></Table.Head>
					</Table.Row>
				</Table.Header>
				<Table.Body>
					{#if !listing.value || !view.rows}
						<TableSkeleton {columns} />
					{:else}
						{@const catalog = listing.value.catalog}
						{#each view.rows as row (row.fullName)}
							{@const sandbox = row.sandbox}
							{@const image = choiceLabel(catalog.images, sandbox.image)}
							{@const size = choiceLabel(catalog.sizes, sandbox.size)}
							{@const inFlight = pending.get(row.fullName)}
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
												aria-label="{row.fullName} details"
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
								<Table.Cell>
									{#if inFlight}
										<Badge variant="outline">
											<Spinner role="presentation" aria-label={undefined} aria-hidden="true" />
											{powers[inFlight].pending}
										</Badge>
									{:else}
										<SandboxStateBadge status={sandbox} detail />
									{/if}
								</Table.Cell>
								<Table.Cell class={wide}>{image}</Table.Cell>
								<Table.Cell class={wide}>{size}</Table.Cell>
								<Table.Cell class={wide}><Timestamp value={sandbox.createdAt} /></Table.Cell>
								<Table.Cell class="w-0 text-right">
									<DropdownMenu.Root>
										<DropdownMenu.Trigger>
											{#snippet child({ props })}
												<Button
													variant="ghost"
													size="icon"
													aria-label="Actions for {row.fullName}"
													{...props}
												>
													<EllipsisIcon />
												</Button>
											{/snippet}
										</DropdownMenu.Trigger>
										<DropdownMenu.Content align="end">
											{@render menu(row, inFlight !== undefined)}
										</DropdownMenu.Content>
									</DropdownMenu.Root>
								</Table.Cell>
							</Table.Row>
						{/each}
					{/if}
				</Table.Body>
			</Table.Root>
		</div>
	{/if}

	{#if failure}
		<Alert.Root variant="destructive">
			<CircleAlertIcon />
			<Alert.Title>{failure.title}</Alert.Title>
			<Alert.Description class="first-letter:uppercase">{failure.message}</Alert.Description>
		</Alert.Root>
	{/if}
</div>

{#snippet menu(row: Row, busy: boolean)}
	<DropdownMenu.Item>
		{#snippet child({ props })}
			<a href="/admin/sandboxes/{row.sandbox.owner.handle}/{row.sandbox.name}" {...props}>
				Details
			</a>
		{/snippet}
	</DropdownMenu.Item>
	{#each row.powers as power (power)}
		<DropdownMenu.Item disabled={busy} onSelect={() => act(row, power)}>
			{powers[power].label}
		</DropdownMenu.Item>
	{/each}
	{#if row.deletable}
		<DropdownMenu.Item variant="destructive" disabled={busy} onSelect={() => confirmDelete(row)}>
			Delete sandbox…
		</DropdownMenu.Item>
	{/if}
{/snippet}

{#if doomed}
	{@const row = doomed}
	<ConfirmDialog
		bind:open={confirming}
		title="Delete {row.fullName}?"
		action="Delete sandbox"
		confirmName={row.sandbox.name}
		onconfirm={() => remove(row)}
	>
		This deletes {row.sandbox.owner.handle}'s sandbox {row.sandbox.name} and everything in its home directory.
		It cannot be undone.
	</ConfirmDialog>
{/if}
