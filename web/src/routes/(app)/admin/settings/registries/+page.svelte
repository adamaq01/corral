<script lang="ts">
	import EllipsisIcon from '@lucide/svelte/icons/ellipsis';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import * as settings from '#lib/api/settings.js';
	import ConfirmDialog from '#lib/components/confirm-dialog.svelte';
	import ListSkeleton from '#lib/components/list-skeleton.svelte';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import ConfigBadge from '#lib/components/settings/config-badge.svelte';
	import RegistryDialog from '#lib/components/settings/registry-dialog.svelte';
	import SaveForm from '#lib/components/settings/save-form.svelte';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import { SettingsForm } from '#lib/settings-form.svelte.js';
	import {
		put,
		refusal,
		registriesChange,
		registriesView,
		without,
		type RegistryEntry
	} from '#lib/settings.js';

	const form = new SettingsForm(
		async () => registriesView(await settings.registries()),
		(view) => settings.setRegistries(registriesChange(view)),
		'Saved the registries'
	);

	type Shown =
		{ dialog: 'registry'; registry: RegistryEntry | null } | { dialog: 'remove'; registry: string };

	/** The dialog shown, kept while it closes. */
	let shown = $state<Shown | null>(null);
	let open = $state(false);

	function show(dialog: Shown) {
		shown = dialog;
		open = true;
	}

	const passwords = {
		set: { label: 'Password set', variant: 'secondary' },
		unset: { label: 'Variable not set', variant: 'destructive' }
	} as const;

	function password(registry: RegistryEntry) {
		if (registry.passwordSet === null) {
			return null;
		}
		return passwords[registry.passwordSet ? 'set' : 'unset'];
	}

	const wide = 'hidden sm:table-cell';
</script>

{#if form.loaded.current.state === 'failed'}
	<LoadFailed
		title="Loading the registries failed"
		error={form.loaded.current.error}
		onretry={() => form.loaded.reload()}
	/>
{:else if !form.edited}
	<ListSkeleton what="the registries" />
{:else}
	{@const view = form.edited}
	<SaveForm {form} what="the registries">
		<Card.Root>
			<Card.Header>
				<Card.Title><h2>Registries</h2></Card.Title>
				<Card.Description>
					Where Corral logs in to pull sandbox images and read their sizes. Each password stays in
					the environment variable it names, where Corral runs.
				</Card.Description>
				<Card.Action>
					<Button variant="outline" onclick={() => show({ dialog: 'registry', registry: null })}>
						<PlusIcon />Add registry
					</Button>
				</Card.Action>
			</Card.Header>
			<Card.Content>
				{#if view.registries.length === 0}
					<p class="text-sm text-muted-foreground">
						None yet. Corral pulls images without logging in until you add one.
					</p>
				{:else}
					<div class="rounded-lg border">
						<Table.Root aria-label="Registries">
							<Table.Header>
								<Table.Row>
									<Table.Head>Registry</Table.Head>
									<Table.Head class={wide}>Username</Table.Head>
									<Table.Head>Password</Table.Head>
									<Table.Head class={wide}>Plain HTTP</Table.Head>
									<Table.Head><span class="sr-only">Actions</span></Table.Head>
								</Table.Row>
							</Table.Header>
							<Table.Body>
								{#each view.registries as row (`${row.state} ${row.key}`)}
									<Table.Row>
										<Table.Cell class="whitespace-normal">
											<div class="grid min-w-0 gap-1">
												<span class="font-mono text-xs break-all">{row.key}</span>
												{#if row.state !== 'shadowed'}
													<span class="text-xs text-muted-foreground sm:hidden">
														{row.entry.username ?? 'No login'}{row.entry.plainHttp
															? ', plain HTTP'
															: ''}
													</span>
												{/if}
												<ConfigBadge state={row.state} />
											</div>
										</Table.Cell>
										{#if row.state === 'shadowed'}
											<Table.Cell colspan={3} class="whitespace-normal text-muted-foreground">
												The config file sets this registry, so this one is not used.
											</Table.Cell>
										{:else}
											{@const status = password(row.entry)}
											<Table.Cell class={wide}>
												{#if row.entry.username}
													{row.entry.username}
												{:else}
													<span class="text-muted-foreground">None</span>
												{/if}
											</Table.Cell>
											<Table.Cell class="whitespace-normal">
												{#if row.entry.passwordEnv}
													<div class="grid justify-items-start gap-1">
														<span class="font-mono text-xs break-all">{row.entry.passwordEnv}</span>
														{#if status}
															<Badge variant={status.variant}>{status.label}</Badge>
														{/if}
													</div>
												{:else}
													<span class="text-muted-foreground">None</span>
												{/if}
											</Table.Cell>
											<Table.Cell class={wide}>{row.entry.plainHttp ? 'Yes' : 'No'}</Table.Cell>
										{/if}
										<Table.Cell class="w-0 text-right">
											{#if row.state !== 'locked'}
												{@render actions(row.key, row.state === 'editable' ? row.entry : null)}
											{/if}
										</Table.Cell>
									</Table.Row>
								{/each}
							</Table.Body>
						</Table.Root>
					</div>
				{/if}
			</Card.Content>
		</Card.Root>
	</SaveForm>

	{#if shown?.dialog === 'registry'}
		<RegistryDialog
			bind:open
			registry={shown.registry}
			refuse={(host) => refusal(view.registries, host, 'listed')}
			onsave={(registry) => (view.registries = put(view.registries, registry.registry, registry))}
		/>
	{:else if shown?.dialog === 'remove'}
		{@const host = shown.registry}
		<ConfirmDialog
			bind:open
			title="Remove {host}?"
			action="Remove registry"
			confirmName={host}
			onconfirm={async () => {
				view.registries = without(view.registries, host);
			}}
		>
			Saving the registries then removes {host}, and Corral pulls from it without logging in.
		</ConfirmDialog>
	{/if}
{/if}

{#snippet actions(host: string, registry: RegistryEntry | null)}
	<DropdownMenu.Root>
		<DropdownMenu.Trigger>
			{#snippet child({ props })}
				<Button variant="ghost" size="icon" aria-label="Actions for {host}" {...props}>
					<EllipsisIcon />
				</Button>
			{/snippet}
		</DropdownMenu.Trigger>
		<DropdownMenu.Content align="end">
			{#if registry}
				<DropdownMenu.Item onSelect={() => show({ dialog: 'registry', registry })}>
					Edit
				</DropdownMenu.Item>
			{/if}
			<DropdownMenu.Item
				variant="destructive"
				onSelect={() => show({ dialog: 'remove', registry: host })}
			>
				Remove
			</DropdownMenu.Item>
		</DropdownMenu.Content>
	</DropdownMenu.Root>
{/snippet}
