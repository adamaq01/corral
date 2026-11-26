<script lang="ts" generics="T extends { label: string; enabled: boolean }">
	import EllipsisIcon from '@lucide/svelte/icons/ellipsis';
	import type { Snippet } from 'svelte';
	import ConfigBadge from '#lib/components/settings/config-badge.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import { Switch } from '#lib/components/ui/switch/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import type { Row } from '#lib/settings.js';

	let {
		kind,
		rows = $bindable(),
		selected = $bindable(),
		selectedLocked,
		detail,
		onedit,
		onremove
	}: {
		kind: 'image' | 'size';
		rows: Row<T>[];
		/** The default's key. */
		selected: string;
		/** Whether the config file sets the defaults. */
		selectedLocked: boolean;
		/** What an entry is made from: its reference, or its resources. */
		detail: Snippet<[T]>;
		onedit: (entry: T) => void;
		onremove: (key: string) => void;
	} = $props();

	const columns = {
		image: { table: 'Images', entry: 'Image', detail: 'Reference' },
		size: { table: 'Sizes', entry: 'Size', detail: 'Resources' }
	};
	const words = $derived(columns[kind]);
	const wide = 'hidden sm:table-cell';
</script>

<div class="rounded-lg border">
	<Table.Root aria-label={words.table}>
		<Table.Header>
			<Table.Row>
				<Table.Head>{words.entry}</Table.Head>
				<Table.Head class={wide}>{words.detail}</Table.Head>
				<Table.Head>
					<span class="flex flex-wrap items-center gap-2">
						Default
						<ConfigBadge state={selectedLocked ? 'locked' : 'editable'} />
					</span>
				</Table.Head>
				<Table.Head>Enabled</Table.Head>
				<Table.Head><span class="sr-only">Actions</span></Table.Head>
			</Table.Row>
		</Table.Header>
		<Table.Body>
			{#each rows as row (`${row.state} ${row.key}`)}
				<Table.Row>
					{#if row.state === 'shadowed'}
						<Table.Cell>
							<div class="grid min-w-0 gap-1">
								<span class="font-mono text-xs break-all">{row.key}</span>
								<ConfigBadge state={row.state} />
							</div>
						</Table.Cell>
						<Table.Cell class="{wide} whitespace-normal text-muted-foreground">
							The config file sets this key, so this {kind} is not used.
						</Table.Cell>
						<Table.Cell></Table.Cell>
						<Table.Cell></Table.Cell>
						<Table.Cell class="w-0 text-right">
							{@render actions(row.key, null)}
						</Table.Cell>
					{:else}
						<Table.Cell class="whitespace-normal">
							<div class="grid min-w-0 gap-1">
								<span class="font-medium">{row.entry.label}</span>
								<span class="font-mono text-xs break-all text-muted-foreground">{row.key}</span>
								<span class="text-xs text-muted-foreground sm:hidden">
									{@render detail(row.entry)}
								</span>
								<ConfigBadge state={row.state} />
								{#if row.state === 'builtin'}
									<span class="text-xs text-muted-foreground">
										Corral offers this until you add your own.
									</span>
								{/if}
							</div>
						</Table.Cell>
						<Table.Cell class="{wide} whitespace-normal">{@render detail(row.entry)}</Table.Cell>
						<Table.Cell>
							<input
								type="radio"
								name="default-{kind}"
								value={row.key}
								bind:group={selected}
								disabled={selectedLocked || !row.entry.enabled}
								aria-label="Default {kind}: {row.key}"
								class="size-4 rounded-full accent-primary outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50"
							/>
						</Table.Cell>
						<Table.Cell>
							{#if row.state === 'editable'}
								<Switch bind:checked={row.entry.enabled} aria-label="Enable {row.key}" />
							{:else}
								<span class="text-muted-foreground">{row.entry.enabled ? 'Yes' : 'No'}</span>
							{/if}
						</Table.Cell>
						<Table.Cell class="w-0 text-right">
							{#if row.state === 'editable'}
								{@render actions(row.key, row.entry)}
							{/if}
						</Table.Cell>
					{/if}
				</Table.Row>
			{/each}
		</Table.Body>
	</Table.Root>
</div>

{#snippet actions(key: string, entry: T | null)}
	<DropdownMenu.Root>
		<DropdownMenu.Trigger>
			{#snippet child({ props })}
				<Button variant="ghost" size="icon" aria-label="Actions for {key}" {...props}>
					<EllipsisIcon />
				</Button>
			{/snippet}
		</DropdownMenu.Trigger>
		<DropdownMenu.Content align="end">
			{#if entry}
				<DropdownMenu.Item onSelect={() => onedit(entry)}>Edit</DropdownMenu.Item>
			{/if}
			<DropdownMenu.Item variant="destructive" onSelect={() => onremove(key)}>
				Remove
			</DropdownMenu.Item>
		</DropdownMenu.Content>
	</DropdownMenu.Root>
{/snippet}
