<script lang="ts">
	import ChevronLeftIcon from '@lucide/svelte/icons/chevron-left';
	import SearchXIcon from '@lucide/svelte/icons/search-x';
	import { goto } from '$app/navigation';
	import { failureMessage } from '#lib/api/client.js';
	import { environment } from '#lib/api/environment.js';
	import type { EnvironmentEntry, ValueInput } from '#lib/api/generated/index.js';
	import ConfirmDialog from '#lib/components/confirm-dialog.svelte';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import SetValueDialog from '#lib/components/set-value-dialog.svelte';
	import StoredValue from '#lib/components/stored-value.svelte';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Empty from '#lib/components/ui/empty/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { Spinner } from '#lib/components/ui/spinner/index.js';
	import { where } from '#lib/environment.js';
	import { Remote } from '#lib/remote.svelte.js';

	let { id }: { id: string } = $props();

	const entry = new Remote(() => environment.get(id));

	let replacing = $state(false);
	let deleting = $state(false);
	let saving = $state(false);
	let saved = $state<{ ok: boolean; message: string } | null>(null);

	async function setFlags(current: EnvironmentEntry, change: Partial<EnvironmentEntry>) {
		saving = true;
		saved = null;
		try {
			const changed = await environment.change(id, {
				template: change.template ?? current.template,
				applyToNew: change.applyToNew ?? current.applyToNew
			});
			entry.update(() => changed);
			saved = { ok: true, message: 'Saved' };
		} catch (failure) {
			saved = { ok: false, message: failureMessage(failure) };
			await entry.reload();
		} finally {
			saving = false;
		}
	}

	async function replace(current: EnvironmentEntry, value: ValueInput) {
		const changed = await environment.change(id, {
			template: current.template,
			applyToNew: current.applyToNew,
			value
		});
		entry.update(() => changed);
	}

	async function remove() {
		await environment.remove(id);
		await goto('/environment', { replace: true });
	}
</script>

<svelte:head>
	<title>{entry.value ? where(entry.value) : 'Environment'} · Corral</title>
</svelte:head>

<div class="grid grid-cols-1 gap-6">
	<a
		href="/environment"
		class="-ml-1 inline-flex w-fit items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
	>
		<ChevronLeftIcon class="size-4" />
		Environment
	</a>

	{#if entry.current.state === 'loading'}
		<div class="grid gap-4" role="status" aria-label="Loading the entry">
			<Skeleton class="h-8 w-64" />
			<Skeleton class="h-40 rounded-xl" />
		</div>
	{:else if entry.current.state === 'failed' && entry.current.status === 404}
		<h1 class="sr-only">Not found</h1>
		<Empty.Root class="border">
			<Empty.Header>
				<Empty.Media variant="icon"><SearchXIcon /></Empty.Media>
				<Empty.Title>No such variable or file</Empty.Title>
				<Empty.Description>It may have been deleted.</Empty.Description>
			</Empty.Header>
			<Empty.Content>
				<Button href="/environment" variant="outline">See your environment</Button>
			</Empty.Content>
		</Empty.Root>
	{:else if entry.current.state === 'failed'}
		<h1 class="sr-only">Environment</h1>
		<LoadFailed
			title="Loading the entry failed"
			error={entry.current.error}
			onretry={() => entry.reload()}
		/>
	{:else}
		{@const current = entry.current.value}
		{@const variable = current.kind === 'variable'}
		<header class="grid gap-1">
			<div class="flex flex-wrap items-center gap-2">
				<h1 class="font-mono text-2xl font-semibold tracking-tight break-all">{where(current)}</h1>
				{#if current.secret}
					<Badge variant="secondary">Secret</Badge>
				{/if}
				{#if current.template}
					<Badge variant="outline">Template</Badge>
				{/if}
			</div>
			<p class="text-sm text-muted-foreground">
				{variable
					? 'An environment variable of each SSH session.'
					: 'A file written into the home at each start, replacing what is there.'}
			</p>
		</header>

		<Card.Root>
			<Card.Header>
				<Card.Title>Value</Card.Title>
				<Card.Description>
					{current.secret
						? 'Secret: kept in the secret store and never shown again.'
						: 'Shown in full, since it is not secret.'}
				</Card.Description>
			</Card.Header>
			<Card.Content>
				<StoredValue value={current.value} kind={current.kind} />
			</Card.Content>
			<Card.Footer>
				<Button variant="outline" onclick={() => (replacing = true)}>
					{current.secret ? 'Replace value' : 'Edit value'}
				</Button>
			</Card.Footer>
		</Card.Root>

		<Card.Root>
			<Card.Header>
				<Card.Title>Settings</Card.Title>
				<Card.Description>Saved as soon as you change them.</Card.Description>
			</Card.Header>
			<Card.Content class="grid gap-3">
				<label class="flex items-start gap-2 text-sm">
					<input
						type="checkbox"
						class="mt-0.5 size-4 accent-primary"
						checked={current.applyToNew}
						disabled={saving}
						onchange={(event) => setFlags(current, { applyToNew: event.currentTarget.checked })}
					/>
					<span>
						<span class="font-medium">Apply to new sandboxes</span>
						<span class="block text-muted-foreground">
							Each sandbox you create from now on gets it. Existing sandboxes keep what they have.
						</span>
					</span>
				</label>
				{#if !variable}
					<label class="flex items-start gap-2 text-sm">
						<input
							type="checkbox"
							class="mt-0.5 size-4 accent-primary"
							checked={current.template}
							disabled={saving}
							onchange={(event) => setFlags(current, { template: event.currentTarget.checked })}
						/>
						<span>
							<span class="font-medium">Template</span>
							<span class="block text-muted-foreground">
								Each <code class="font-mono text-xs">{'{{ NAME }}'}</code> in the text takes the value
								of the variable NAME attached to the same sandbox.
							</span>
						</span>
					</label>
				{/if}
				<div class="min-h-5" aria-live="polite">
					{#if saving}
						<Spinner />
					{:else if saved}
						<p
							class={[
								'text-sm first-letter:uppercase',
								saved.ok ? 'text-muted-foreground' : 'text-destructive'
							]}
						>
							{saved.message}
						</p>
					{/if}
				</div>
			</Card.Content>
		</Card.Root>

		<Card.Root>
			<Card.Header>
				<Card.Title>Sandboxes</Card.Title>
				<Card.Description>
					Attach it to a sandbox, or give it a value there alone, on the sandbox's page.
				</Card.Description>
			</Card.Header>
			<Card.Content>
				{#if current.sandboxes.length === 0}
					<p class="text-sm text-muted-foreground">Attached to no sandbox.</p>
				{:else}
					<ul class="flex flex-wrap gap-2" aria-label="Attached to">
						{#each current.sandboxes as sandbox (sandbox)}
							<li>
								<Button href="/sandboxes/{sandbox}" variant="outline" size="sm">{sandbox}</Button>
							</li>
						{/each}
					</ul>
				{/if}
			</Card.Content>
		</Card.Root>

		<Card.Root class="ring-destructive/30">
			<Card.Header>
				<Card.Title>Delete</Card.Title>
				<Card.Description>Detaches it from every sandbox and deletes its value.</Card.Description>
			</Card.Header>
			<Card.Footer>
				<Button variant="destructive" onclick={() => (deleting = true)}>
					Delete {variable ? 'variable' : 'file'}
				</Button>
			</Card.Footer>
		</Card.Root>

		<SetValueDialog
			bind:open={replacing}
			title={current.secret ? `Replace ${where(current)}` : `Edit ${where(current)}`}
			description={current.secret
				? "The new value takes the old one's place, which is gone for good. Sandboxes with a value of their own keep it."
				: 'Sandboxes with a value of their own keep it.'}
			submit="Save value"
			kind={current.kind}
			secret={current.secret}
			text={current.value.shown === 'text' ? current.value.text : ''}
			onsave={(value) => replace(current, value)}
		/>

		<ConfirmDialog
			bind:open={deleting}
			title="Delete {where(current)}?"
			action="Delete {variable ? 'variable' : 'file'}"
			onconfirm={remove}
		>
			<p>
				Its value is gone for good, with any value a sandbox has of its own. Sandboxes lose it when
				they next start.
			</p>
		</ConfirmDialog>
	{/if}
</div>
