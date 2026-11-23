<script lang="ts">
	import FileTextIcon from '@lucide/svelte/icons/file-text';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import VariableIcon from '@lucide/svelte/icons/variable';
	import { failureMessage } from '#lib/api/client.js';
	import { environment } from '#lib/api/environment.js';
	import type { AttachedEntry, ValueInput } from '#lib/api/generated/index.js';
	import AttachEntryDialog from '#lib/components/attach-entry-dialog.svelte';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import SetValueDialog from '#lib/components/set-value-dialog.svelte';
	import StoredValue from '#lib/components/stored-value.svelte';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { where } from '#lib/environment.js';
	import { Remote } from '#lib/remote.svelte.js';

	let { sandbox, onchanged }: { sandbox: string; onchanged: () => Promise<void> } = $props();

	const attached = new Remote(() => environment.ofSandbox(sandbox));
	const registry = new Remote(environment.list);
	const unattached = $derived(
		(registry.value ?? []).filter(
			(entry) => !attached.value?.some((held) => held.entry.id === entry.id)
		)
	);

	let attaching = $state(false);
	let overriding = $state<AttachedEntry | null>(null);
	let settingValue = $state(false);
	let failure = $state<{ id: string; message: string } | null>(null);

	async function changed() {
		await Promise.all([attached.reload(), registry.reload(), onchanged()]);
	}

	async function act(held: AttachedEntry, run: () => Promise<unknown>) {
		failure = null;
		try {
			await run();
		} catch (error) {
			failure = { id: held.entry.id, message: failureMessage(error) };
		}
		await changed();
	}

	function openOverride(held: AttachedEntry) {
		overriding = held;
		settingValue = true;
	}

	async function saveOverride(value: ValueInput) {
		if (overriding) {
			await environment.override(sandbox, overriding.entry.id, value);
			await changed();
		}
	}
</script>

<Card.Root>
	<Card.Header>
		<Card.Title>Environment</Card.Title>
		<Card.Description>
			The variables and files this sandbox gets at each start. A value set here is this sandbox's
			alone; the rest follow their entry on
			<a class="underline underline-offset-4" href="/environment">Environment</a>.
		</Card.Description>
		<Card.Action>
			<Button
				variant="outline"
				size="sm"
				disabled={!registry.value || !attached.value}
				onclick={() => (attaching = true)}
			>
				<PlusIcon />
				Attach
			</Button>
		</Card.Action>
	</Card.Header>
	<Card.Content>
		{#if attached.current.state === 'loading'}
			<div class="grid gap-2" role="status" aria-label="Loading the sandbox's environment">
				<Skeleton class="h-14 rounded-lg" />
			</div>
		{:else if attached.current.state === 'failed'}
			<LoadFailed
				title="Loading the sandbox's environment failed"
				error={attached.current.error}
				onretry={() => attached.reload()}
			/>
		{:else if attached.current.value.length === 0}
			<p class="text-sm text-muted-foreground">Nothing is attached to this sandbox.</p>
		{:else}
			<ul class="grid divide-y" aria-label="Attached to {sandbox}">
				{#each attached.current.value as held (held.entry.id)}
					{@const entry = held.entry}
					{@const Icon = entry.kind === 'variable' ? VariableIcon : FileTextIcon}
					<li class="grid gap-3 py-3 first:pt-0 last:pb-0 sm:grid-cols-[1fr_auto]">
						<div class="flex min-w-0 gap-3">
							<Icon class="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
							<div class="grid min-w-0 gap-1">
								<p class="flex flex-wrap items-center gap-2 text-sm font-medium">
									<a class="font-mono break-all hover:underline" href="/environment/{entry.id}">
										{where(entry)}
									</a>
									{#if entry.secret}
										<Badge variant="secondary">Secret</Badge>
									{/if}
									{#if entry.template}
										<Badge variant="outline">Template</Badge>
									{/if}
									{#if held.override}
										<Badge>This sandbox's value</Badge>
									{/if}
								</p>
								<StoredValue value={held.override ?? entry.value} kind={entry.kind} />
								{#if failure?.id === entry.id}
									<p class="text-sm text-destructive first-letter:uppercase" role="alert">
										{failure.message}
									</p>
								{/if}
							</div>
						</div>
						<div class="flex flex-wrap items-start gap-2 sm:justify-end">
							<Button variant="outline" size="sm" onclick={() => openOverride(held)}>
								{held.override ? 'Change' : 'Set a value here'}<span class="sr-only">
									for {where(entry)}
								</span>
							</Button>
							{#if held.override}
								<Button
									variant="outline"
									size="sm"
									onclick={() => act(held, () => environment.clearOverride(sandbox, entry.id))}
								>
									Use the shared value<span class="sr-only"> for {where(entry)}</span>
								</Button>
							{/if}
							<Button
								variant="ghost"
								size="sm"
								onclick={() => act(held, () => environment.detach(sandbox, entry.id))}
							>
								Detach<span class="sr-only"> {where(entry)}</span>
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
	{@const current = overriding.override}
	<SetValueDialog
		bind:open={settingValue}
		title="{where(entry)} in {sandbox}"
		description="This sandbox gets this value in place of the shared one, from its next start.{entry.secret
			? ' Like the shared value, it is secret.'
			: ''}"
		submit="Save value"
		kind={entry.kind}
		secret={entry.secret}
		text={current?.shown === 'text' ? current.text : ''}
		onsave={saveOverride}
	/>
{/if}
