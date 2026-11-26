<script lang="ts">
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import DownloadIcon from '@lucide/svelte/icons/download';
	import FileTextIcon from '@lucide/svelte/icons/file-text';
	import InfoIcon from '@lucide/svelte/icons/info';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import SearchIcon from '@lucide/svelte/icons/search';
	import UploadIcon from '@lucide/svelte/icons/upload';
	import VariableIcon from '@lucide/svelte/icons/variable';
	import { toast } from 'svelte-sonner';
	import { goto } from '$app/navigation';
	import { setup } from '#lib/api/setup.js';
	import type { EntryKind, SetupEntry } from '#lib/api/generated/index.js';
	import EntryDialog from '#lib/components/entry-dialog.svelte';
	import ImportVariablesDialog from '#lib/components/import-variables-dialog.svelte';
	import ListSkeleton from '#lib/components/list-skeleton.svelte';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import PageHeader from '#lib/components/page-header.svelte';
	import RefreshStatus from '#lib/components/refresh-status.svelte';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Empty from '#lib/components/ui/empty/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import * as Item from '#lib/components/ui/item/index.js';
	import * as NativeSelect from '#lib/components/ui/native-select/index.js';
	import { SHARING, described, download, sections, toDotenv, where } from '#lib/setup.js';
	import { Remote, keepFresh } from '#lib/remote.svelte.js';

	const list = new Remote(setup.list);
	keepFresh(list);

	let adding = $state(false);
	let importing = $state(false);
	let kind = $state<EntryKind>('variable');
	let search = $state('');
	let show = $state<'all' | EntryKind | 'secret'>('all');

	const parts = $derived(sections(list.value ?? []));

	function shown(entry: SetupEntry): boolean {
		const matches =
			where(entry).toLowerCase().includes(search.trim().toLowerCase()) ||
			entry.sandboxes.some((used) => used.sandbox.includes(search.trim().toLowerCase()));
		const kept = show === 'all' || (show === 'secret' ? entry.secret : entry.kind === show);
		return matches && kept;
	}

	function add(which: EntryKind) {
		kind = which;
		adding = true;
	}

	async function created(entry: SetupEntry) {
		toast.success(`Added ${where(entry)}`);
		await goto(`/setup/${entry.id}`);
	}

	async function imported(count: number) {
		if (count > 0) {
			toast.success(`Imported ${count} ${count === 1 ? 'variable' : 'variables'}`);
		}
		await list.reload();
	}

	function exportVariables(entries: SetupEntry[]) {
		download('corral.env', toDotenv(entries));
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
		<Button variant="outline" onclick={() => (importing = true)}>
			<UploadIcon />
			Import .env
		</Button>
	</div>
{/snippet}

{#snippet noneYet()}
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
{/snippet}

{#snippet listed(label: string, entries: SetupEntry[])}
	{@const matching = entries.filter(shown)}
	{#if matching.length === 0}
		<p class="text-sm text-muted-foreground">Nothing matches.</p>
	{:else}
		<ul class="grid gap-2" aria-label={label}>
			{#each matching as entry (entry.id)}
				{@const Icon = entry.kind === 'variable' ? VariableIcon : FileTextIcon}
				<li>
					<Item.Root variant="outline">
						{#snippet child({ props })}
							<a href="/setup/{entry.id}" {...props}>
								<Item.Media variant="icon">
									<Icon class="text-muted-foreground" />
								</Item.Media>
								<Item.Content class="min-w-0">
									<Item.Title class="max-w-full flex-wrap font-mono break-all">
										{where(entry)}
										{#if entry.shared}
											<Badge variant="outline">{SHARING[entry.shared].badge}</Badge>
										{/if}
										{#if entry.shadows}
											<Badge variant="outline">Yours, in place of the shared one</Badge>
										{/if}
										{#if entry.secret}
											<Badge variant="secondary">Secret</Badge>
										{/if}
										{#if entry.template}
											<Badge variant="outline">Template</Badge>
										{/if}
										{#if entry.duplicate}
											<Badge variant="destructive">Duplicate</Badge>
										{/if}
									</Item.Title>
									<Item.Description class="break-all">
										{described(entry)}
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
{/snippet}

<div class="grid grid-cols-1 gap-6">
	<PageHeader
		title="Setup"
		description="Variables and files your sandboxes come with, such as API keys and tool settings."
	>
		{#snippet actions()}
			{#if parts.yours.length > 0}
				{@render addButtons(true)}
			{/if}
		{/snippet}
	</PageHeader>

	<Alert.Root>
		<InfoIcon />
		<Alert.Title>Changes reach a sandbox when it next starts</Alert.Title>
		<Alert.Description>
			A sandbox gets the entries attached to it, which you choose here or on its page. A sleeping
			sandbox gets the current values when it wakes; a running one keeps what it started with until
			it restarts, and Sandboxes marks it until then. Files are written at each start, replacing
			what is at their path.
		</Alert.Description>
	</Alert.Root>

	{#if list.current.state === 'loading'}
		<ListSkeleton what="your setup" />
	{:else if list.current.state === 'failed'}
		<LoadFailed
			title="Loading your setup failed"
			error={list.current.error}
			onretry={() => list.reload()}
		/>
	{:else if list.current.value.length === 0}
		{@render noneYet()}
	{:else}
		<div class="grid gap-3">
			<div class="flex flex-wrap items-center gap-2">
				<div class="relative min-w-48 flex-1">
					<SearchIcon
						class="absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
						aria-hidden="true"
					/>
					<Input
						type="search"
						placeholder="Find by name, path or sandbox"
						aria-label="Find by name, path or sandbox"
						class="pl-8"
						bind:value={search}
					/>
				</div>
				<NativeSelect.Root aria-label="Show" bind:value={show}>
					<NativeSelect.Option value="all">All</NativeSelect.Option>
					<NativeSelect.Option value="variable">Variables</NativeSelect.Option>
					<NativeSelect.Option value="file">Files</NativeSelect.Option>
					<NativeSelect.Option value="secret">Secret</NativeSelect.Option>
				</NativeSelect.Root>
				{#if parts.yours.some((entry) => entry.kind === 'variable')}
					<Button variant="ghost" onclick={() => exportVariables(parts.yours)}>
						<DownloadIcon />
						Export .env
					</Button>
				{/if}
			</div>
			<RefreshStatus remote={list} />
			{#if parts.shared.length === 0}
				{@render listed('Setup', parts.yours)}
			{:else}
				<h2 class="mt-3 text-lg font-semibold tracking-tight">Shared</h2>
				{@render listed('Shared', parts.shared)}
				<h2 class="mt-3 text-lg font-semibold tracking-tight">Yours</h2>
				{#if parts.yours.length === 0}
					{@render noneYet()}
				{:else}
					{@render listed('Yours', parts.yours)}
				{/if}
			{/if}
		</div>
	{/if}
</div>

<EntryDialog bind:open={adding} start={{ kind }} onsaved={created} />
<ImportVariablesDialog bind:open={importing} existing={parts.yours} onimported={imported} />
