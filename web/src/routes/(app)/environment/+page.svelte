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
	import { environment } from '#lib/api/environment.js';
	import type { EntryKind, EnvironmentEntry } from '#lib/api/generated/index.js';
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
	import { download, lineCount, summary, toDotenv, where } from '#lib/environment.js';
	import { Remote, keepFresh } from '#lib/remote.svelte.js';

	const list = new Remote(environment.list);
	keepFresh(list);

	let adding = $state(false);
	let importing = $state(false);
	let kind = $state<EntryKind>('variable');
	let search = $state('');
	let show = $state<'all' | EntryKind | 'secret'>('all');

	const shown = $derived(
		(list.value ?? []).filter((entry) => {
			const matches =
				where(entry).toLowerCase().includes(search.trim().toLowerCase()) ||
				entry.sandboxes.some((used) => used.sandbox.includes(search.trim().toLowerCase()));
			const kept = show === 'all' || (show === 'secret' ? entry.secret : entry.kind === show);
			return matches && kept;
		})
	);

	function add(which: EntryKind) {
		kind = which;
		adding = true;
	}

	async function created(entry: EnvironmentEntry) {
		toast.success(`Added ${where(entry)}`);
		await goto(`/environment/${entry.id}`);
	}

	async function imported(count: number) {
		if (count > 0) {
			toast.success(`Imported ${count} ${count === 1 ? 'variable' : 'variables'}`);
		}
		await list.reload();
	}

	function exportVariables(entries: EnvironmentEntry[]) {
		download('corral.env', toDotenv(entries));
	}

	function described(entry: EnvironmentEntry): string {
		const lines = entry.value.shown === 'text' ? lineCount(entry.value.text) : 0;
		const value =
			entry.kind === 'variable' || entry.value.shown !== 'text'
				? summary(entry.value)
				: `${lines} ${lines === 1 ? 'line' : 'lines'}`;
		const on =
			entry.sandboxes.length === 0
				? 'no sandbox'
				: entry.sandboxes.map((used) => used.sandbox).join(', ');
		const attached = `${value ? 'attached' : 'Attached'} to ${on}`;
		return [value, attached, entry.applyToNew ? 'new sandboxes get it' : null]
			.filter(Boolean)
			.join(' · ');
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
			A sandbox gets the entries attached to it, which you choose here or on its page. A sleeping
			sandbox gets the current values when it wakes; a running one keeps what it started with until
			it restarts, and Sandboxes marks it until then. Files are written at each start, replacing
			what is at their path.
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
		{@const entries = list.current.value}
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
				{#if entries.some((entry) => entry.kind === 'variable')}
					<Button variant="ghost" onclick={() => exportVariables(entries)}>
						<DownloadIcon />
						Export .env
					</Button>
				{/if}
			</div>
			<RefreshStatus remote={list} />
			{#if shown.length === 0}
				<p class="text-sm text-muted-foreground">Nothing matches.</p>
			{:else}
				<ul class="grid gap-2" aria-label="Environment">
					{#each shown as entry (entry.id)}
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
		</div>
	{/if}
</div>

<EntryDialog bind:open={adding} start={{ kind }} onsaved={created} />
<ImportVariablesDialog bind:open={importing} existing={list.value ?? []} onimported={imported} />
