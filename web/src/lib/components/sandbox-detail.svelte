<script lang="ts" module>
	const moment = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' });
</script>

<script lang="ts">
	import ChevronLeftIcon from '@lucide/svelte/icons/chevron-left';
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import MoonIcon from '@lucide/svelte/icons/moon';
	import PowerIcon from '@lucide/svelte/icons/power';
	import RotateCwIcon from '@lucide/svelte/icons/rotate-cw';
	import SearchXIcon from '@lucide/svelte/icons/search-x';
	import StarIcon from '@lucide/svelte/icons/star';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import VariableIcon from '@lucide/svelte/icons/variable';
	import { toast } from 'svelte-sonner';
	import { goto } from '$app/navigation';
	import { opksshLogin } from '#lib/api/auth.js';
	import { failureMessage } from '#lib/api/client.js';
	import { sandboxes } from '#lib/api/sandboxes.js';
	import ConfirmDialog from '#lib/components/confirm-dialog.svelte';
	import CopySnippet from '#lib/components/copy-snippet.svelte';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import RefreshStatus from '#lib/components/refresh-status.svelte';
	import SandboxSetup from '#lib/components/sandbox-setup.svelte';
	import SandboxSleep from '#lib/components/sandbox-sleep.svelte';
	import SandboxStateBadge from '#lib/components/sandbox-state-badge.svelte';
	import StateAnnouncer from '#lib/components/state-announcer.svelte';
	import Timestamp from '#lib/components/timestamp.svelte';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Empty from '#lib/components/ui/empty/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { Spinner } from '#lib/components/ui/spinner/index.js';
	import {
		OPKSSH_ALIAS,
		OPKSSH_LOGIN_TIMEOUT_SECS,
		connectSnippets,
		hostAlias,
		opksshSnippets
	} from '#lib/connect.js';
	import { choiceLabel } from '#lib/catalog.js';
	import { Remote, keepFresh } from '#lib/remote.svelte.js';
	import { powerInFlight, runPower, unsettled } from '#lib/sandbox-actions.svelte.js';
	import { states, type Power } from '#lib/sandbox-status.js';

	let { name, handle }: { name: string; handle: string } = $props();

	const sandbox = new Remote(() => sandboxes.get(name));
	const catalog = new Remote(sandboxes.catalog);
	const opkssh = new Remote(opksshLogin);

	keepFresh(sandbox, () => (sandbox.value ? unsettled(sandbox.value) : false));

	const powers: Record<Power, { label: string; pending: string; icon: typeof PowerIcon }> = {
		wake: { label: 'Wake', pending: 'Waking…', icon: PowerIcon },
		stop: { label: 'Sleep', pending: 'Going to sleep…', icon: MoonIcon },
		restart: { label: 'Restart', pending: 'Restarting…', icon: RotateCwIcon }
	};

	const inFlight = $derived(powerInFlight(name));
	const starting = $derived(inFlight === 'wake' || inFlight === 'restart');
	let failure = $state<{ action: string; message: string } | null>(null);
	let stillStarting = $state(false);
	let defaulting = $state(false);
	let deleting = $state(false);

	let elapsed = $state(0);
	$effect(() => {
		if (!starting) {
			return;
		}
		const since = Date.now();
		elapsed = 0;
		const timer = setInterval(() => (elapsed = Math.floor((Date.now() - since) / 1000)), 1000);
		return () => clearInterval(timer);
	});

	async function power(power: Power) {
		failure = null;
		stillStarting = false;
		try {
			stillStarting = !(await runPower(name, power));
		} catch (error) {
			failure = { action: powers[power].label, message: failureMessage(error) };
		}
		await sandbox.reload();
	}

	async function makeDefault() {
		defaulting = true;
		failure = null;
		try {
			await sandboxes.makeDefault(name);
			toast.success(`${name} is now your default sandbox`);
		} catch (error) {
			failure = { action: 'Make default', message: failureMessage(error) };
		} finally {
			defaulting = false;
		}
		await sandbox.reload();
	}

	async function remove() {
		await sandboxes.remove(name);
		toast.success(`Deleted ${name}`);
		await goto('/sandboxes', { replace: true });
	}

	let cancelling = $state(false);
	let cancelFailure = $state<string | null>(null);

	async function cancelDeletion() {
		cancelling = true;
		cancelFailure = null;
		try {
			await sandboxes.cancelDeletion(name);
			toast.success(`Cancelled the deletion of ${name}`);
		} catch (error) {
			cancelFailure = failureMessage(error);
		} finally {
			cancelling = false;
		}
		await sandbox.reload();
	}
</script>

<svelte:head>
	<title>{name} · Corral</title>
</svelte:head>

{#snippet powerButton(action: Power, variant: 'default' | 'outline')}
	{@const Icon = powers[action].icon}
	<Button {variant} disabled={inFlight !== null} onclick={() => power(action)}>
		{#if inFlight === action}
			<Spinner role="presentation" aria-label={undefined} aria-hidden="true" />
			{powers[action].pending}
		{:else}
			<Icon />
			{powers[action].label}
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
			<p class="text-sm text-muted-foreground">
				SSH username
				<code class="font-mono text-xs wrap-anywhere text-foreground">
					{sandbox.value.ssh.username}
				</code>
			</p>
		{/if}
		<RefreshStatus remote={sandbox} />
	</header>

	<StateAnnouncer sandboxes={sandbox.value ? [sandbox.value] : undefined} />

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

		{#if current.deleteAt}
			<Alert.Root variant="destructive">
				<Trash2Icon />
				<Alert.Title>
					Deletion scheduled for {moment.format(new Date(current.deleteAt))}
				</Alert.Title>
				<Alert.Description class="grid gap-3">
					<p>
						<code class="font-mono text-xs">corral delete</code> in this sandbox scheduled its deletion,
						with everything in its home directory. It stays asleep until then, unless you cancel.
					</p>
					{#if cancelFailure}
						<p class="first-letter:uppercase" role="alert">{cancelFailure}</p>
					{/if}
					<div>
						<Button variant="outline" size="sm" disabled={cancelling} onclick={cancelDeletion}>
							{#if cancelling}
								<Spinner role="presentation" aria-label={undefined} aria-hidden="true" />
								Cancelling…
							{:else}
								Cancel deletion
							{/if}
						</Button>
					</div>
				</Alert.Description>
			</Alert.Root>
		{/if}

		{#if current.staleSetup}
			<Alert.Root>
				<VariableIcon />
				<Alert.Title>Restart to apply your setup changes</Alert.Title>
				<Alert.Description>
					Its setup changed after this sandbox started. Until it restarts, its SSH sessions and home
					get what it started with.
				</Alert.Description>
			</Alert.Root>
		{/if}

		{#if presentation.manageable && !current.deleteAt}
			{@const available = current.staleSetup ? (['restart', 'stop'] as const) : presentation.powers}
			<div class="grid gap-3">
				<div class="flex flex-wrap gap-2">
					{#each available as action, index (action)}
						{@render powerButton(action, index === 0 ? 'default' : 'outline')}
					{/each}
					{#if !current.isDefault}
						<Button variant="outline" disabled={defaulting} onclick={makeDefault}>
							{#if defaulting}
								<Spinner role="presentation" aria-label={undefined} aria-hidden="true" />
								Saving…
							{:else}
								<StarIcon />
								Make default
							{/if}
						</Button>
					{/if}
				</div>
				{#if starting}
					<p class="text-sm text-muted-foreground">
						<span role="status">Starting {name}. This usually takes under a minute.</span>
						<span aria-hidden="true">{elapsed} s so far.</span>
					</p>
				{:else if stillStarting && current.state === 'starting'}
					<p class="text-sm text-muted-foreground" role="status">
						{name} is still starting. This page updates when it is up.
					</p>
				{/if}
				{#if failure}
					<Alert.Root variant="destructive">
						<CircleAlertIcon />
						<Alert.Title>{failure.action} failed</Alert.Title>
						<Alert.Description class="first-letter:uppercase">{failure.message}</Alert.Description>
					</Alert.Root>
				{/if}
			</div>
		{/if}

		<Card.Root>
			<Card.Header>
				<Card.Title>Connect</Card.Title>
				<Card.Description>
					Any SSH client works with a key from <a
						class="underline underline-offset-4"
						href="/account#keys"
					>
						SSH keys
					</a>.
					{current.deleteAt
						? 'Connections are refused until you cancel the deletion.'
						: 'Connecting wakes a sleeping sandbox.'}
				</Card.Description>
			</Card.Header>
			<Card.Content class="grid grid-cols-1 gap-4">
				{#each connectSnippets(current, handle) as snippet (snippet.label)}
					<CopySnippet label={snippet.label} text={snippet.text} />
				{/each}
				<p class="text-sm text-muted-foreground">
					With the SSH config block in <code class="font-mono text-xs">~/.ssh/config</code>,
					<code class="font-mono text-xs">ssh {hostAlias(current)}</code> is enough.
					<a class="underline underline-offset-4" href="https://herdr.dev/">Herdr</a>, the app for
					coding agents, shows the agents running in this sandbox once you add it with the Herdr
					command. Herdr staying connected does not keep the sandbox awake, and opening a terminal
					in it wakes it. Connecting on a terminal to a name you have no sandbox by opens an
					installer that makes it.
				</p>
			</Card.Content>
		</Card.Root>

		{#if opkssh.value}
			{@const login = opkssh.value}
			<Card.Root>
				<Card.Header>
					<Card.Title>Connect with opkssh</Card.Title>
					<Card.Description>
						Instead of registering a key, install
						<a class="underline underline-offset-4" href="https://github.com/openpubkey/opkssh">
							opkssh
						</a>
						and add these to your computer. Your single sign-on login then stands in for a key for {login.lifetimeHours}
						hours at a time.
					</Card.Description>
				</Card.Header>
				<Card.Content class="grid grid-cols-1 gap-4">
					{#each opksshSnippets(login, current.ssh.host) as snippet (snippet.label)}
						<CopySnippet label={snippet.label} text={snippet.text} />
					{/each}
					<p class="text-sm text-muted-foreground">
						Before connecting, ssh renews the certificate when it is missing or about to expire.
						While you are signed in to single sign-on in your browser, that takes a moment and no
						clicks. Otherwise sign in within {OPKSSH_LOGIN_TIMEOUT_SECS} seconds, or run
						<code class="font-mono text-xs">opkssh login {OPKSSH_ALIAS}</code> first.
					</p>
				</Card.Content>
			</Card.Root>
		{:else if opkssh.current.state === 'failed'}
			<p class="flex flex-wrap items-center gap-x-2 text-sm text-muted-foreground">
				<span class="first-letter:uppercase">
					Checking for opkssh sign-in failed: {opkssh.current.error}.
				</span>
				<Button variant="link" size="sm" class="h-auto p-0" onclick={() => opkssh.reload()}>
					Retry
				</Button>
			</p>
		{/if}

		{#if presentation.manageable && !current.deleteAt}
			<SandboxSleep sandbox={current} onchanged={() => sandbox.reload()} />
		{/if}

		<SandboxSetup sandbox={name} onchanged={() => sandbox.reload()} />

		{#if presentation.manageable}
			<Card.Root class="ring-destructive/30">
				<Card.Header>
					<Card.Title>Delete this sandbox</Card.Title>
					<Card.Description>Deletes the sandbox and its home directory for good.</Card.Description>
				</Card.Header>
				<Card.Footer class="flex-wrap gap-3">
					<Button
						variant="destructive"
						disabled={inFlight !== null}
						onclick={() => (deleting = true)}
					>
						Delete sandbox
					</Button>
					{#if inFlight !== null}
						<p class="text-sm text-muted-foreground">
							You can delete it once {powers[inFlight].label} finishes.
						</p>
					{/if}
				</Card.Footer>
			</Card.Root>
		{/if}
	{/if}
</div>

<ConfirmDialog
	bind:open={deleting}
	title="Delete {name}?"
	action="Delete sandbox"
	confirmName={name}
	onconfirm={remove}
>
	<p>This deletes the sandbox and everything in its home directory. It cannot be undone.</p>
	<p>
		If you added it to Herdr, remove it from Herdr before you create a sandbox named
		<span class="font-mono text-foreground">{name}</span> again: to Herdr, the new sandbox is a different
		one.
	</p>
</ConfirmDialog>
