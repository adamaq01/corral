<script lang="ts">
	import ChevronLeftIcon from '@lucide/svelte/icons/chevron-left';
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import PowerIcon from '@lucide/svelte/icons/power';
	import RotateCwIcon from '@lucide/svelte/icons/rotate-cw';
	import SearchXIcon from '@lucide/svelte/icons/search-x';
	import SquareIcon from '@lucide/svelte/icons/square';
	import StarIcon from '@lucide/svelte/icons/star';
	import { goto } from '$app/navigation';
	import { failureMessage } from '#lib/api/client.js';
	import { sandboxes } from '#lib/api/sandboxes.js';
	import ConfirmDialog from '#lib/components/confirm-dialog.svelte';
	import CopySnippet from '#lib/components/copy-snippet.svelte';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import SandboxStateBadge from '#lib/components/sandbox-state-badge.svelte';
	import Timestamp from '#lib/components/timestamp.svelte';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Empty from '#lib/components/ui/empty/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { Spinner } from '#lib/components/ui/spinner/index.js';
	import { connectSnippets, hostAlias } from '#lib/connect.js';
	import { choiceLabel } from '#lib/catalog.js';
	import { Remote, pollWhile } from '#lib/remote.svelte.js';
	import { states, type Power } from '#lib/sandbox-status.js';

	let { name, handle }: { name: string; handle: string } = $props();

	const sandbox = new Remote(() => sandboxes.get(name));
	const catalog = new Remote(sandboxes.catalog);

	type Action = Power | 'default';

	const actions: Record<
		Action,
		{ label: string; pending: string; icon: typeof PowerIcon; run: () => Promise<unknown> }
	> = {
		wake: { label: 'Wake', pending: 'Waking…', icon: PowerIcon, run: () => sandboxes.wake(name) },
		stop: {
			label: 'Stop',
			pending: 'Stopping…',
			icon: SquareIcon,
			run: () => sandboxes.stop(name)
		},
		restart: {
			label: 'Restart',
			pending: 'Restarting…',
			icon: RotateCwIcon,
			run: () => sandboxes.restart(name)
		},
		default: {
			label: 'Make default',
			pending: 'Saving…',
			icon: StarIcon,
			run: () => sandboxes.makeDefault(name)
		}
	};

	let running = $state<Action | null>(null);
	let failure = $state<{ action: Action; message: string } | null>(null);
	let deleting = $state(false);

	pollWhile(
		() => running === 'wake' || (sandbox.value ? states[sandbox.value.state].settling : false),
		sandbox
	);

	async function act(action: Action) {
		running = action;
		failure = null;
		try {
			await actions[action].run();
		} catch (error) {
			failure = { action, message: failureMessage(error) };
		} finally {
			running = null;
		}
		await sandbox.reload();
	}

	async function remove() {
		await sandboxes.remove(name);
		await goto('/sandboxes', { replace: true });
	}
</script>

<svelte:head>
	<title>{name} · Corral</title>
</svelte:head>

{#snippet actionButton(action: Action, variant: 'default' | 'outline')}
	{@const Icon = actions[action].icon}
	<Button {variant} disabled={running !== null} onclick={() => act(action)}>
		{#if running === action}
			<Spinner role="presentation" aria-label={undefined} aria-hidden="true" />
			{actions[action].pending}
		{:else}
			<Icon />
			{actions[action].label}
		{/if}
	</Button>
{/snippet}

<div class="grid grid-cols-1 gap-6">
	<a
		href="/sandboxes"
		class="-ml-1 inline-flex w-fit items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
	>
		<ChevronLeftIcon class="size-4" />
		Sandboxes
	</a>

	<header class="grid gap-1">
		<div class="flex flex-wrap items-center gap-2">
			<h1 class="text-2xl font-semibold tracking-tight break-all">{name}</h1>
			{#if sandbox.value}
				<SandboxStateBadge status={sandbox.value} />
				{#if sandbox.value.isDefault}
					<Badge variant="secondary">Default</Badge>
				{/if}
			{/if}
		</div>
		{#if sandbox.value}
			<p class="text-sm text-muted-foreground">
				{choiceLabel(catalog.value?.images, sandbox.value.image)} · {choiceLabel(
					catalog.value?.sizes,
					sandbox.value.size
				)} · Created <Timestamp value={sandbox.value.createdAt} />
			</p>
		{/if}
	</header>

	{#if sandbox.current.state === 'loading'}
		<div class="grid gap-4" role="status" aria-label="Loading the sandbox">
			<Skeleton class="h-5 w-64" />
			<Skeleton class="h-56 rounded-xl" />
		</div>
	{:else if sandbox.current.state === 'failed' && sandbox.current.status === 404}
		<Empty.Root class="border">
			<Empty.Header>
				<Empty.Media variant="icon"><SearchXIcon /></Empty.Media>
				<Empty.Title>No sandbox named {name}</Empty.Title>
				<Empty.Description>It may have been deleted.</Empty.Description>
			</Empty.Header>
			<Empty.Content>
				<Button href="/sandboxes" variant="outline">See your sandboxes</Button>
			</Empty.Content>
		</Empty.Root>
	{:else if sandbox.current.state === 'failed'}
		<LoadFailed
			title="Loading the sandbox failed"
			error={sandbox.current.error}
			onretry={() => sandbox.reload()}
		/>
	{:else}
		{@const current = sandbox.current.value}
		{@const presentation = states[current.state]}
		{#if current.state === 'failed'}
			<Alert.Root variant="destructive">
				<CircleAlertIcon />
				<Alert.Title>The sandbox failed</Alert.Title>
				<Alert.Description class="break-words first-letter:uppercase">
					{current.detail}
				</Alert.Description>
			</Alert.Root>
		{/if}

		{#if failure}
			<Alert.Root variant="destructive">
				<CircleAlertIcon />
				<Alert.Title>{actions[failure.action].label} failed</Alert.Title>
				<Alert.Description class="first-letter:uppercase">{failure.message}</Alert.Description>
			</Alert.Root>
		{/if}

		{#if presentation.manageable}
			<div class="flex flex-wrap gap-2">
				{#each presentation.powers as power, index (power)}
					{@render actionButton(power, index === 0 ? 'default' : 'outline')}
				{/each}
				{#if !current.isDefault}
					{@render actionButton('default', 'outline')}
				{/if}
			</div>
		{/if}

		<Card.Root>
			<Card.Header>
				<Card.Title>Connect</Card.Title>
				<Card.Description>
					Any SSH client works with a key from <a class="underline underline-offset-4" href="/keys">
						SSH keys
					</a>. Connecting wakes a sleeping sandbox.
				</Card.Description>
			</Card.Header>
			<Card.Content class="grid grid-cols-1 gap-4">
				{#each connectSnippets(current) as snippet (snippet.label)}
					<CopySnippet label={snippet.label} text={snippet.text} />
				{/each}
				<p class="text-sm text-muted-foreground">
					With the SSH config block in <code class="font-mono text-xs">~/.ssh/config</code>,
					<code class="font-mono text-xs">ssh {hostAlias(current)}</code> is enough.
					{#if current.isDefault}
						As your default sandbox, it also answers to
						<code class="font-mono text-xs wrap-anywhere">
							ssh -p {current.ssh.port}
							{handle}@{current.ssh.host}
						</code>.
					{/if}
				</p>
			</Card.Content>
		</Card.Root>

		{#if presentation.manageable}
			<Card.Root class="ring-destructive/30">
				<Card.Header>
					<Card.Title>Delete this sandbox</Card.Title>
					<Card.Description>Deletes the sandbox and its home directory for good.</Card.Description>
				</Card.Header>
				<Card.Footer>
					<Button variant="destructive" onclick={() => (deleting = true)}>Delete sandbox</Button>
				</Card.Footer>
			</Card.Root>
		{/if}
	{/if}
</div>

<ConfirmDialog
	bind:open={deleting}
	title="Delete {name}?"
	action="Delete sandbox"
	onconfirm={remove}
>
	<p>This deletes the sandbox and everything in its home directory. It cannot be undone.</p>
	<p>
		If you added it to Herdr, remove the machine from Herdr before you create a sandbox named
		<span class="font-mono text-foreground">{name}</span> again: to Herdr, the new sandbox is a different
		machine.
	</p>
</ConfirmDialog>
