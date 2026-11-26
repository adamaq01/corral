<script lang="ts">
	import FileTextIcon from '@lucide/svelte/icons/file-text';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import VariableIcon from '@lucide/svelte/icons/variable';
	import { toast } from 'svelte-sonner';
	import { setup } from '#lib/api/setup.js';
	import type { AttachedEntry, ValueInput } from '#lib/api/generated/index.js';
	import AttachEntryDialog from '#lib/components/attach-entry-dialog.svelte';
	import ConfirmDialog from '#lib/components/confirm-dialog.svelte';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import SetValueDialog from '#lib/components/set-value-dialog.svelte';
	import StoredValue from '#lib/components/stored-value.svelte';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { baseName, where } from '#lib/setup.js';
	import { Remote } from '#lib/remote.svelte.js';

	let { sandbox, onchanged }: { sandbox: string; onchanged: () => Promise<void> } = $props();

	const attached = new Remote(() => setup.ofSandbox(sandbox));
	const registry = new Remote(setup.list);
	const unattached = $derived(
		(registry.value ?? []).filter(
			(entry) => !attached.value?.some((held) => held.entry.id === entry.id)
		)
	);

	let attaching = $state(false);
	let overriding = $state<AttachedEntry | null>(null);
	let settingValue = $state(false);
	let clearing = $state<AttachedEntry | null>(null);
	let detaching = $state<AttachedEntry | null>(null);

	async function changed() {
		await Promise.all([attached.reload(), registry.reload(), onchanged()]);
	}

	function openOverride(held: AttachedEntry) {
		overriding = held;
		settingValue = true;
	}

	async function saveOverride(value: ValueInput) {
		if (overriding) {
			await setup.override(sandbox, overriding.entry.id, value);
			toast.success(`Saved the override of ${where(overriding.entry)} for ${sandbox}`);
			await changed();
		}
	}

	async function clearOverride(held: AttachedEntry) {
		await setup.clearOverride(sandbox, held.entry.id);
		toast.success(`${sandbox} takes ${where(held.entry)}'s own value again`);
		await changed();
	}

	async function detach(held: AttachedEntry) {
		await setup.detach(sandbox, held.entry.id);
		toast.success(`Detached ${where(held.entry)} from ${sandbox}`);
		await changed();
	}
</script>

<Card.Root>
	<Card.Header>
		<Card.Title>Setup</Card.Title>
		<Card.Description>
			The variables and files this sandbox gets at each start, from
			<a class="underline underline-offset-4" href="/setup">Setup</a>. An override gives one of them
			another value here alone.
		</Card.Description>
		{#if attached.value?.length}
			<Card.Action>
				<Button
					variant="outline"
					size="sm"
					disabled={!registry.value}
					onclick={() => (attaching = true)}
				>
					<PlusIcon />
					Attach
				</Button>
			</Card.Action>
		{/if}
	</Card.Header>
	<Card.Content>
		{#if attached.current.state === 'loading'}
			<div class="grid gap-2" role="status" aria-label="Loading the sandbox's setup">
				<Skeleton class="h-14 rounded-lg" />
			</div>
		{:else if attached.current.state === 'failed'}
			<LoadFailed
				title="Loading the sandbox's setup failed"
				error={attached.current.error}
				onretry={() => attached.reload()}
			/>
		{:else if attached.current.value.length === 0}
			<div
				class="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-dashed p-4"
			>
				<p class="text-sm text-muted-foreground">
					Nothing is attached to this sandbox. {(registry.value?.length ?? 0) > 0
						? `You have ${registry.value?.length} to choose from.`
						: 'Add an API key, a token or a config file on Setup first.'}
				</p>
				{#if (registry.value?.length ?? 0) > 0}
					<Button variant="outline" size="sm" onclick={() => (attaching = true)}>
						<PlusIcon />
						Attach variables and files
					</Button>
				{:else}
					<Button variant="outline" size="sm" href="/setup">Go to Setup</Button>
				{/if}
			</div>
		{:else}
			<ul class="grid divide-y" aria-label="Attached to {sandbox}">
				{#each attached.current.value as held (held.entry.id)}
					{@const entry = held.entry}
					{@const Icon = entry.kind === 'variable' ? VariableIcon : FileTextIcon}
					{@const unresolved =
						entry.sandboxes.find((used) => used.sandbox === sandbox)?.unresolved ?? []}
					<li class="grid gap-3 py-3 first:pt-0 last:pb-0 sm:grid-cols-[1fr_auto]">
						<div class="flex min-w-0 gap-3">
							<Icon class="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
							<div class="grid min-w-0 flex-1 gap-1.5">
								<p class="flex flex-wrap items-center gap-2 text-sm font-medium">
									<a class="font-mono break-all hover:underline" href="/setup/{entry.id}">
										{where(entry)}
									</a>
									{#if entry.secret}
										<Badge variant="secondary">Secret</Badge>
									{/if}
									{#if entry.template}
										<Badge variant="outline">Template</Badge>
									{/if}
									{#if held.override}
										<Badge>Override</Badge>
									{/if}
								</p>
								<StoredValue
									value={held.override ?? entry.value}
									kind={entry.kind}
									label="{where(entry)} in {sandbox}"
									file={baseName(entry.target)}
								/>
								{#if unresolved.length > 0}
									<p class="text-sm text-destructive">
										Cannot start: the template refers to {unresolved.join(', ')}, not attached here.
									</p>
								{/if}
							</div>
						</div>
						<div class="flex flex-wrap items-start gap-2 sm:justify-end">
							<Button
								variant="outline"
								size="sm"
								aria-label="{held.override
									? 'Change override of'
									: 'Override for this sandbox:'} {where(entry)}"
								onclick={() => openOverride(held)}
							>
								{held.override ? 'Change override' : 'Override for this sandbox'}
							</Button>
							{#if held.override}
								<Button
									variant="outline"
									size="sm"
									aria-label="Remove override of {where(entry)}"
									onclick={() => (clearing = held)}
								>
									Remove override
								</Button>
							{/if}
							<Button
								variant="ghost"
								size="sm"
								aria-label="Detach {where(entry)}"
								onclick={() => (detaching = held)}
							>
								Detach
							</Button>
						</div>
					</li>
				{/each}
			</ul>
		{/if}
	</Card.Content>
</Card.Root>

<AttachEntryDialog bind:open={attaching} {sandbox} entries={unattached} onAttached={changed} />

{#if overriding}
	{@const entry = overriding.entry}
	<SetValueDialog
		bind:open={settingValue}
		title="Override {where(entry)} for {sandbox}"
		description="{sandbox} gets this value in place of the entry's own, from its next start.{entry.secret
			? ' Like the entry, it is secret.'
			: ''}"
		submit="Save override"
		kind={entry.kind}
		secret={entry.secret}
		name={entry.kind === 'variable' ? entry.target : ''}
		current={overriding.override}
		onsave={saveOverride}
	/>
{/if}

<ConfirmDialog
	bind:open={() => clearing !== null, (next) => !next && (clearing = null)}
	title="Remove the override of {clearing ? where(clearing.entry) : ''}?"
	action="Remove override"
	onconfirm={() => clearOverride(clearing!)}
>
	<p>
		{sandbox} takes the entry's own value again from its next start. The override is gone for good.
	</p>
</ConfirmDialog>

<ConfirmDialog
	bind:open={() => detaching !== null, (next) => !next && (detaching = null)}
	title="Detach {detaching ? where(detaching.entry) : ''} from {sandbox}?"
	action="Detach"
	onconfirm={() => detach(detaching!)}
>
	<p>
		{sandbox} loses it at its next start{detaching?.override
			? ', with its override, which is gone for good'
			: ''}.{detaching?.entry.kind === 'file'
			? ' Corral then removes the file, unless it was changed in the sandbox.'
			: ''}
	</p>
</ConfirmDialog>
