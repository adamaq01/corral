<script lang="ts">
	import EllipsisIcon from '@lucide/svelte/icons/ellipsis';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import VariableIcon from '@lucide/svelte/icons/variable';
	import { toast } from 'svelte-sonner';
	import type { EntryKind, SharedEntry } from '#lib/api/generated/index.js';
	import { removeSharedEntry, sharedSetup } from '#lib/api/settings.js';
	import ConfirmDialog from '#lib/components/confirm-dialog.svelte';
	import ListSkeleton from '#lib/components/list-skeleton.svelte';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import RefreshStatus from '#lib/components/refresh-status.svelte';
	import SharedEntryDialog from '#lib/components/settings/shared-entry-dialog.svelte';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import * as Empty from '#lib/components/ui/empty/index.js';
	import { Spinner } from '#lib/components/ui/spinner/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import { Remote, keepFresh } from '#lib/remote.svelte.js';
	import { SHARING, noun, where } from '#lib/setup.js';

	const list = new Remote(sharedSetup);
	keepFresh(list);

	let refreshing = $state(false);

	async function refresh() {
		refreshing = true;
		await list.reload();
		refreshing = false;
	}

	type Shown =
		| { dialog: 'entry'; entry: SharedEntry | null; kind: EntryKind }
		| { dialog: 'delete'; entry: SharedEntry };

	/** The dialog shown, kept while it closes. */
	let shown = $state<Shown | null>(null);
	let open = $state(false);

	function show(dialog: Shown) {
		shown = dialog;
		open = true;
	}

	async function saved(entry: SharedEntry, added: boolean) {
		toast.success(`${added ? 'Added' : 'Saved'} ${where(entry)}`);
		await list.reload();
	}

	async function remove(entry: SharedEntry) {
		try {
			await removeSharedEntry(entry.id);
			toast.success(`Deleted ${where(entry)}`);
		} finally {
			await list.reload();
		}
	}

	/** Which sandboxes have the entry, in one line, for the phone where its columns are hidden. */
	function reach(entry: SharedEntry): string {
		if (entry.shared === 'required') {
			return 'Every sandbox has it';
		}
		return [
			`Attached to ${entry.attached} ${entry.attached === 1 ? 'sandbox' : 'sandboxes'}`,
			entry.applyToNew ? 'new sandboxes get it' : null,
			entry.shadowedBy.length > 0 ? `shadowed by ${entry.shadowedBy.join(', ')}` : null
		]
			.filter(Boolean)
			.join(', ');
	}

	/** What deleting the entry loses. */
	function losses(entry: SharedEntry): string {
		const value =
			entry.shared === 'required'
				? 'Its value is gone for good.'
				: 'Its value is gone for good, with every override.';
		const file =
			entry.kind === 'file' ? ', and Corral removes the file unless it was changed there' : '';
		if (entry.shared === 'required') {
			return `${value} Every sandbox loses it at its next start${file}.`;
		}
		if (entry.attached === 0) {
			return value;
		}
		const sandboxes =
			entry.attached === 1
				? 'The sandbox attached to it loses it at its next start'
				: `The ${entry.attached} sandboxes attached to it lose it at their next start`;
		return `${value} ${sandboxes}${file}.`;
	}

	const wide = 'hidden sm:table-cell';
</script>

{#snippet addButtons()}
	<div class="flex flex-wrap gap-2">
		<Button onclick={() => show({ dialog: 'entry', entry: null, kind: 'variable' })}>
			<PlusIcon />Add variable
		</Button>
		<Button variant="outline" onclick={() => show({ dialog: 'entry', entry: null, kind: 'file' })}>
			<PlusIcon />Add file
		</Button>
	</div>
{/snippet}

{#if list.current.state === 'failed'}
	<LoadFailed
		title="Loading the shared setup failed"
		error={list.current.error}
		onretry={() => list.reload()}
	/>
{:else}
	{@const entries = list.value}
	<Card.Root>
		<Card.Header>
			<Card.Title><h2>Shared setup</h2></Card.Title>
			<Card.Description>
				Variables and files for everyone's sandboxes. A required entry reaches every sandbox at its
				next start; members attach an offered one, override it, or replace it with their own.
			</Card.Description>
		</Card.Header>
		<Card.Content class="grid gap-3">
			<div class="flex flex-wrap items-center gap-x-3 gap-y-2">
				{#if entries?.length}
					{@render addButtons()}
				{/if}
				<div class="ms-auto flex flex-wrap items-center gap-x-3 gap-y-2">
					<RefreshStatus remote={list} time />
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
			</div>
			{#if entries === undefined}
				<ListSkeleton what="the shared setup" />
			{:else if entries.length === 0}
				<Empty.Root>
					<Empty.Header>
						<Empty.Media variant="icon"><VariableIcon /></Empty.Media>
						<Empty.Title>No shared entries yet</Empty.Title>
						<Empty.Description>
							Add what every sandbox should come with, such as a gateway address.
						</Empty.Description>
					</Empty.Header>
					<Empty.Content>{@render addButtons()}</Empty.Content>
				</Empty.Root>
			{:else}
				<div class="rounded-lg border">
					<Table.Root aria-label="Shared setup">
						<Table.Header>
							<Table.Row>
								<Table.Head>Entry</Table.Head>
								<Table.Head>Shared as</Table.Head>
								<Table.Head class={wide}>New sandboxes</Table.Head>
								<Table.Head class={wide}>Attached</Table.Head>
								<Table.Head class={wide}>Shadowed by</Table.Head>
								<Table.Head><span class="sr-only">Actions</span></Table.Head>
							</Table.Row>
						</Table.Header>
						<Table.Body>
							{#each entries as entry (entry.id)}
								{@const required = entry.shared === 'required'}
								<Table.Row>
									<Table.Cell class="whitespace-normal">
										<div class="grid min-w-0 gap-1">
											<span class="font-mono text-xs break-all">{where(entry)}</span>
											{#if entry.secret || entry.template || entry.executable}
												<div class="flex flex-wrap gap-1">
													{#if entry.secret}
														<Badge variant="secondary">Secret</Badge>
													{/if}
													{#if entry.template}
														<Badge variant="outline">Template</Badge>
													{/if}
													{#if entry.executable}
														<Badge variant="outline">Executable</Badge>
													{/if}
												</div>
											{/if}
											<span class="text-xs text-muted-foreground sm:hidden">{reach(entry)}</span>
										</div>
									</Table.Cell>
									<Table.Cell>{SHARING[entry.shared].badge}</Table.Cell>
									<Table.Cell class={wide}>
										{#if required}
											<span class="text-muted-foreground">Always</span>
										{:else}
											{entry.applyToNew ? 'Yes' : 'No'}
										{/if}
									</Table.Cell>
									<Table.Cell class={wide}>
										{#if required}
											<span class="text-muted-foreground">Every sandbox</span>
										{:else}
											{entry.attached}
										{/if}
									</Table.Cell>
									<Table.Cell class="{wide} whitespace-normal">
										{#if entry.shadowedBy.length > 0}
											{entry.shadowedBy.join(', ')}
										{:else}
											<span class="text-muted-foreground">Nobody</span>
										{/if}
									</Table.Cell>
									<Table.Cell class="w-0 text-right">{@render actions(entry)}</Table.Cell>
								</Table.Row>
							{/each}
						</Table.Body>
					</Table.Root>
				</div>
			{/if}
		</Card.Content>
	</Card.Root>

	{#if shown?.dialog === 'entry'}
		{@const added = shown.entry === null}
		<SharedEntryDialog
			bind:open
			entry={shown.entry}
			kind={shown.kind}
			others={entries ?? []}
			onsaved={(entry) => saved(entry, added)}
		/>
	{:else if shown?.dialog === 'delete'}
		{@const entry = shown.entry}
		<ConfirmDialog
			bind:open
			title="Delete {where(entry)}?"
			action="Delete {noun(entry.kind)}"
			confirmName={where(entry)}
			onconfirm={() => remove(entry)}
		>
			<p>{losses(entry)}</p>
		</ConfirmDialog>
	{/if}
{/if}

{#snippet actions(entry: SharedEntry)}
	<DropdownMenu.Root>
		<DropdownMenu.Trigger>
			{#snippet child({ props })}
				<Button variant="ghost" size="icon" aria-label="Actions for {where(entry)}" {...props}>
					<EllipsisIcon />
				</Button>
			{/snippet}
		</DropdownMenu.Trigger>
		<DropdownMenu.Content align="end">
			<DropdownMenu.Item onSelect={() => show({ dialog: 'entry', entry, kind: entry.kind })}>
				Edit
			</DropdownMenu.Item>
			<DropdownMenu.Item variant="destructive" onSelect={() => show({ dialog: 'delete', entry })}>
				Delete…
			</DropdownMenu.Item>
		</DropdownMenu.Content>
	</DropdownMenu.Root>
{/snippet}
