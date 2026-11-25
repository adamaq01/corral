<script lang="ts">
	import ChevronLeftIcon from '@lucide/svelte/icons/chevron-left';
	import CopyPlusIcon from '@lucide/svelte/icons/copy-plus';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import SearchXIcon from '@lucide/svelte/icons/search-x';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import { toast } from 'svelte-sonner';
	import { goto } from '$app/navigation';
	import { failureMessage } from '#lib/api/client.js';
	import { environment } from '#lib/api/environment.js';
	import type { EnvironmentEntry, TemplatePreview, ValueInput } from '#lib/api/generated/index.js';
	import { sandboxes } from '#lib/api/sandboxes.js';
	import ConfirmDialog from '#lib/components/confirm-dialog.svelte';
	import EntryDialog from '#lib/components/entry-dialog.svelte';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import SetValueDialog from '#lib/components/set-value-dialog.svelte';
	import StoredValue from '#lib/components/stored-value.svelte';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Empty from '#lib/components/ui/empty/index.js';
	import * as NativeSelect from '#lib/components/ui/native-select/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { Spinner } from '#lib/components/ui/spinner/index.js';
	import { baseName, noun, settings, where } from '#lib/environment.js';
	import { Remote } from '#lib/remote.svelte.js';

	let { id }: { id: string } = $props();

	const entry = new Remote(() => environment.get(id));
	const owned = new Remote(sandboxes.list);

	let editing = $state(false);
	let duplicating = $state(false);
	let replacing = $state(false);
	let deleting = $state(false);
	let detaching = $state<string | null>(null);
	let pending = $state<string | null>(null);
	let failure = $state<{ sandbox: string; message: string } | null>(null);
	let previewIn = $state('');
	let preview = $state<{ sandbox: string; result: TemplatePreview | string } | null>(null);

	async function replace(current: EnvironmentEntry, value: ValueInput) {
		const changed = await environment.change(id, { ...settings(current), value });
		entry.update(() => changed);
		toast.success(`Saved the value of ${where(changed)}`);
	}

	async function edited(changed: EnvironmentEntry) {
		entry.update(() => changed);
		preview = null;
		toast.success(`Saved ${where(changed)}`);
	}

	async function duplicated(created: EnvironmentEntry) {
		toast.success(`Added ${where(created)}`);
		await goto(`/environment/${created.id}`);
	}

	async function act(sandbox: string, run: () => Promise<unknown>, done: string) {
		pending = sandbox;
		failure = null;
		try {
			await run();
			toast.success(done);
		} catch (error) {
			failure = { sandbox, message: failureMessage(error) };
		} finally {
			pending = null;
		}
		await entry.reload();
	}

	async function attachAll(current: EnvironmentEntry, names: string[]) {
		for (const name of names) {
			await act(name, () => environment.attach(name, current.id), `Attached to ${name}`);
			if (failure) {
				return;
			}
		}
	}

	async function detach(sandbox: string) {
		await environment.detach(sandbox, id);
		toast.success(`Detached from ${sandbox}`);
		await entry.reload();
	}

	async function showPreview(sandbox: string) {
		try {
			preview = { sandbox, result: await environment.preview(sandbox, id) };
		} catch (error) {
			preview = { sandbox, result: failureMessage(error) };
		}
	}

	async function remove(current: EnvironmentEntry) {
		await environment.remove(id);
		toast.success(`Deleted ${where(current)}`);
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
		{@const uses = new Map(current.sandboxes.map((used) => [used.sandbox, used]))}
		{@const unattached = (owned.value ?? [])
			.map((sandbox) => sandbox.name)
			.filter((name) => !uses.has(name))}
		<header class="flex flex-wrap items-start justify-between gap-4">
			<div class="grid min-w-0 gap-1">
				<div class="flex flex-wrap items-center gap-2">
					<h1 class="font-mono text-2xl font-semibold tracking-tight wrap-anywhere">
						{#each where(current).split('/') as part, index (index)}{#if index > 0}/<wbr
								/>{/if}{part}{/each}
					</h1>
					{#if current.secret}
						<Badge variant="secondary">Secret</Badge>
					{/if}
					{#if current.template}
						<Badge variant="outline">Template</Badge>
					{/if}
					{#if current.executable}
						<Badge variant="outline">Executable</Badge>
					{/if}
				</div>
				<p class="text-sm text-muted-foreground">
					{variable
						? 'An environment variable of each SSH session of the sandboxes it is attached to.'
						: 'A file Corral writes into the home at each start of the sandboxes it is attached to, replacing what is at its path. Edits made to it inside a sandbox are lost at the next start; change it here instead.'}
				</p>
			</div>
			<div class="flex flex-wrap gap-2">
				<Button variant="outline" onclick={() => (editing = true)}>
					<PencilIcon />
					Edit
				</Button>
				<Button variant="outline" onclick={() => (duplicating = true)}>
					<CopyPlusIcon />
					Duplicate
				</Button>
			</div>
		</header>

		{#if current.duplicate}
			<Alert.Root variant="destructive">
				<TriangleAlertIcon />
				<Alert.Title>
					Another {noun(current.kind)} has this {variable ? 'name' : 'path'}
				</Alert.Title>
				<Alert.Description>
					It was made before each name and path could be used once. Rename this one with Edit, or
					delete it; to give one sandbox another value, override the other one there.
				</Alert.Description>
			</Alert.Root>
		{/if}

		<Card.Root>
			<Card.Header>
				<Card.Title>Value</Card.Title>
				<Card.Description>
					{current.secret
						? 'Kept in the secret store and never shown again.'
						: 'Not secret, so shown in full.'} A sandbox with an override keeps its own.
				</Card.Description>
				<Card.Action>
					<Button variant="outline" size="sm" onclick={() => (replacing = true)}>
						{current.secret ? 'Replace value' : 'Edit value'}
					</Button>
				</Card.Action>
			</Card.Header>
			<Card.Content>
				<StoredValue
					value={current.value}
					kind={current.kind}
					label={where(current)}
					file={baseName(current.target)}
				/>
			</Card.Content>
		</Card.Root>

		{#if current.template}
			{@const attachedNames = current.sandboxes.map((used) => used.sandbox)}
			<Card.Root>
				<Card.Header>
					<Card.Title>Template</Card.Title>
					<Card.Description>
						Each <code class="font-mono">{'{{ NAME }}'}</code> takes the value the variable NAME has in
						the sandbox, which must be attached there for the sandbox to start.
					</Card.Description>
				</Card.Header>
				<Card.Content class="grid gap-4">
					<div class="grid gap-2">
						<p class="text-sm font-medium">Refers to</p>
						{#if current.references.length === 0}
							<p class="text-sm text-muted-foreground">No variable.</p>
						{:else}
							<ul class="flex flex-wrap gap-2" aria-label="Variables it refers to">
								{#each current.references as name (name)}
									<li><Badge variant="outline" class="font-mono">{name}</Badge></li>
								{/each}
							</ul>
						{/if}
					</div>
					{#if attachedNames.length > 0 && !current.secret}
						<form
							class="flex flex-wrap items-end gap-2"
							onsubmit={(event) => {
								event.preventDefault();
								void showPreview(previewIn || attachedNames[0]);
							}}
						>
							<div class="grid gap-1.5">
								<label for="preview-in" class="text-sm font-medium">Preview in</label>
								<NativeSelect.Root
									id="preview-in"
									value={previewIn || attachedNames[0]}
									onchange={(event) => (previewIn = event.currentTarget.value)}
								>
									{#each attachedNames as name (name)}
										<NativeSelect.Option value={name}>{name}</NativeSelect.Option>
									{/each}
								</NativeSelect.Root>
							</div>
							<Button type="submit" variant="outline">Preview</Button>
						</form>
						{#if preview}
							{#if typeof preview.result === 'string'}
								<p class="text-sm text-destructive first-letter:uppercase" role="alert">
									{preview.result}
								</p>
							{:else}
								<div class="grid gap-1.5" aria-live="polite">
									<p class="text-xs text-muted-foreground">
										As {preview.sandbox} gets it at its next start, secret values masked.
									</p>
									<pre
										class="max-h-96 overflow-auto rounded-md border bg-muted/40 px-3 py-2 font-mono text-xs break-all whitespace-pre-wrap">{preview
											.result.text || ' '}</pre>
									{#if preview.result.unresolved.length > 0}
										<p class="text-sm text-destructive">
											{preview.result.unresolved.join(', ')} not attached to {preview.sandbox},
											which cannot start until it is.
										</p>
									{/if}
								</div>
							{/if}
						{/if}
					{:else if current.secret}
						<p class="text-sm text-muted-foreground">A secret template has no preview.</p>
					{/if}
				</Card.Content>
			</Card.Root>
		{/if}

		<Card.Root>
			<Card.Header>
				<Card.Title>Sandboxes</Card.Title>
				<Card.Description>
					{current.applyToNew ? 'Each sandbox you create gets it.' : 'New sandboxes do not get it.'} A
					change reaches a sandbox at its next start.
				</Card.Description>
				{#if unattached.length > 1}
					<Card.Action>
						<Button
							variant="outline"
							size="sm"
							disabled={pending !== null}
							onclick={() => attachAll(current, unattached)}
						>
							Attach to all
						</Button>
					</Card.Action>
				{/if}
			</Card.Header>
			<Card.Content>
				{#if owned.current.state === 'loading'}
					<Skeleton class="h-10 rounded-lg" />
				{:else if owned.current.state === 'failed'}
					<LoadFailed
						title="Loading your sandboxes failed"
						error={owned.current.error}
						onretry={() => owned.reload()}
					/>
				{:else if owned.current.value.length === 0}
					<p class="text-sm text-muted-foreground">
						You have no sandbox yet. {current.applyToNew ? 'The first one you create gets it.' : ''}
					</p>
				{:else}
					<ul class="grid divide-y" aria-label="Your sandboxes">
						{#each owned.current.value as sandbox (sandbox.name)}
							{@const used = uses.get(sandbox.name)}
							<li class="flex flex-wrap items-center gap-3 py-2.5 first:pt-0 last:pb-0">
								<div class="grid min-w-0 flex-1 gap-1">
									<p class="flex flex-wrap items-center gap-2 text-sm">
										<a class="font-medium hover:underline" href="/sandboxes/{sandbox.name}">
											{sandbox.name}
										</a>
										{#if used?.overridden}
											<Badge variant="secondary">Overridden</Badge>
										{/if}
										{#if !used}
											<span class="text-muted-foreground">Not attached</span>
										{/if}
									</p>
									{#if used && used.unresolved.length > 0}
										<p class="text-sm text-destructive">
											Cannot start: {used.unresolved.join(', ')} not attached there.
										</p>
									{/if}
									{#if failure?.sandbox === sandbox.name}
										<p class="text-sm text-destructive first-letter:uppercase" role="alert">
											{failure.message}
										</p>
									{/if}
								</div>
								{#if used}
									<Button
										variant="ghost"
										size="sm"
										disabled={pending !== null}
										onclick={() => (detaching = sandbox.name)}
									>
										Detach<span class="sr-only"> from {sandbox.name}</span>
									</Button>
								{:else}
									<Button
										variant="outline"
										size="sm"
										disabled={pending !== null}
										onclick={() =>
											act(
												sandbox.name,
												() => environment.attach(sandbox.name, current.id),
												`Attached to ${sandbox.name}`
											)}
									>
										{#if pending === sandbox.name}
											<Spinner />
										{/if}
										Attach<span class="sr-only"> to {sandbox.name}</span>
									</Button>
								{/if}
							</li>
						{/each}
					</ul>
				{/if}
			</Card.Content>
		</Card.Root>

		<Card.Root class="ring-destructive/30">
			<Card.Header>
				<Card.Title>Delete</Card.Title>
				<Card.Description>
					Detaches it from every sandbox and deletes its value and every override.
				</Card.Description>
			</Card.Header>
			<Card.Footer>
				<Button variant="destructive" onclick={() => (deleting = true)}>
					Delete {noun(current.kind)}
				</Button>
			</Card.Footer>
		</Card.Root>

		<EntryDialog bind:open={editing} entry={current} onsaved={edited} />
		<EntryDialog
			bind:open={duplicating}
			start={{
				...settings(current),
				target: variable ? `${current.target}_COPY` : `~/${current.target}.copy`
			}}
			onsaved={duplicated}
		/>

		<SetValueDialog
			bind:open={replacing}
			title={current.secret ? `Replace ${where(current)}` : `Edit ${where(current)}`}
			description={current.secret
				? "The new value takes the old one's place, which is gone for good. Sandboxes with an override keep it."
				: 'Sandboxes with an override keep it.'}
			submit="Save value"
			kind={current.kind}
			secret={current.secret}
			name={variable ? current.target : ''}
			current={current.value}
			onsave={(value) => replace(current, value)}
		/>

		<ConfirmDialog
			bind:open={() => detaching !== null, (next) => !next && (detaching = null)}
			title="Detach {where(current)} from {detaching}?"
			action="Detach"
			onconfirm={() => detach(detaching!)}
		>
			<p>
				{detaching} loses it at its next start{uses.get(detaching ?? '')?.overridden
					? ', with its override, which is gone for good'
					: ''}.{variable ? '' : ' Corral then removes the file, unless it was changed there.'}
			</p>
		</ConfirmDialog>

		<ConfirmDialog
			bind:open={deleting}
			title="Delete {where(current)}?"
			action="Delete {noun(current.kind)}"
			confirmName={current.target}
			onconfirm={() => remove(current)}
		>
			<p>
				Its value is gone for good, with every override. Sandboxes lose it when they next start{variable
					? ''
					: ', and Corral removes the file unless it was changed there'}.
			</p>
		</ConfirmDialog>
	{/if}
</div>
