<script lang="ts">
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import { toast } from 'svelte-sonner';
	import { goto } from '$app/navigation';
	import { failureMessage } from '#lib/api/client.js';
	import type { SandboxDefaults } from '#lib/api/generated/index.js';
	import {
		deleteAccount,
		deleteFromInside,
		setDeleteFromInside,
		setSandboxDefaults
	} from '#lib/api/me.js';
	import { sandboxes } from '#lib/api/sandboxes.js';
	import ConfirmDialog from '#lib/components/confirm-dialog.svelte';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import PageHeader from '#lib/components/page-header.svelte';
	import UnsavedGuard from '#lib/components/unsaved-guard.svelte';
	import UserAvatar from '#lib/components/user-avatar.svelte';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Field from '#lib/components/ui/field/index.js';
	import * as NativeSelect from '#lib/components/ui/native-select/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { Spinner } from '#lib/components/ui/spinner/index.js';
	import { Switch } from '#lib/components/ui/switch/index.js';
	import { Remote } from '#lib/remote.svelte.js';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
	const user = $derived(data.user);

	const roles = { member: 'Member', admin: 'Admin' } as const;

	/** The defaults as Corral has them, and as the form shows them. */
	let saved = $state<SandboxDefaults | null>(null);
	let picked = $state<SandboxDefaults | null>(null);
	/** Corral stands in for a pick it no longer offers until the user saves another. */
	let withdrawn = $state(false);
	const catalog = new Remote(async () => {
		const loaded = await sandboxes.catalog();
		saved = { image: loaded.defaultImage, size: loaded.defaultSize };
		picked = { ...saved };
		withdrawn = loaded.defaultsWithdrawn;
		return loaded;
	});
	const dirty = $derived(
		picked !== null &&
			saved !== null &&
			(picked.image !== saved.image || picked.size !== saved.size)
	);
	let saving = $state(false);
	let saveFailure = $state<string | null>(null);

	async function saveDefaults(event: SubmitEvent) {
		event.preventDefault();
		if (!picked) {
			return;
		}
		const sending = { ...picked };
		saving = true;
		saveFailure = null;
		try {
			await setSandboxDefaults(sending);
			saved = sending;
			withdrawn = false;
			toast.success('Saved your defaults. New sandboxes start from these.');
		} catch (failure) {
			saveFailure = failureMessage(failure);
		} finally {
			saving = false;
		}
	}

	function resetDefaults() {
		if (saved) {
			picked = { ...saved };
		}
		saveFailure = null;
	}

	const insideDelete = new Remote(deleteFromInside);
	let switching = $state(false);
	let switchFailure = $state<string | null>(null);

	async function allowInsideDelete(enabled: boolean) {
		switching = true;
		switchFailure = null;
		try {
			await setDeleteFromInside({ enabled });
			insideDelete.update(() => ({ enabled }));
			toast.success(
				enabled
					? 'corral delete now works in your sandboxes'
					: 'corral delete is now refused in your sandboxes'
			);
		} catch (failure) {
			switchFailure = failureMessage(failure);
			await insideDelete.reload();
		} finally {
			switching = false;
		}
	}

	let deleting = $state(false);

	async function remove() {
		await deleteAccount();
		await goto('/sign-in', { replace: true, invalidateAll: true });
	}
</script>

<UnsavedGuard {dirty} what="your sandbox defaults" />

<div class="grid grid-cols-1 gap-6">
	<PageHeader title="Profile" description="Your account." />

	<Card.Root>
		<Card.Content class="flex flex-wrap items-center gap-4">
			<UserAvatar handle={user.handle} picture={user.picture} class="size-16 text-xl" />
			<div class="grid min-w-0 gap-1">
				<div class="flex flex-wrap items-center gap-2">
					<h2 class="text-lg font-semibold break-all">{user.handle}</h2>
					<Badge variant={user.role === 'admin' ? 'default' : 'secondary'}>
						{roles[user.role]}
					</Badge>
				</div>
				<p class="text-sm break-all text-muted-foreground">{user.email ?? 'No email address'}</p>
			</div>
		</Card.Content>
	</Card.Root>

	<Card.Root>
		<Card.Header>
			<Card.Title>New sandboxes</Card.Title>
			<Card.Description>
				The image and size a new sandbox gets unless you pick others. Connecting over SSH as
				<code class="font-mono text-xs wrap-anywhere">&lt;name&gt;.{user.handle}</code>
				with a name you have not used yet makes a sandbox with them too.
				{#if catalog.value?.namedImages}
					A default comes from this list; to start from another image, name it when you create the
					sandbox.
				{/if}
			</Card.Description>
		</Card.Header>
		{#if catalog.current.state === 'failed'}
			<Card.Content>
				<LoadFailed
					title="Loading the catalog failed"
					error={catalog.current.error}
					onretry={() => catalog.reload()}
				/>
			</Card.Content>
		{:else if !catalog.value || !picked}
			<Card.Content class="grid gap-4 sm:grid-cols-2">
				<Skeleton class="h-14" />
				<Skeleton class="h-14" />
			</Card.Content>
		{:else}
			{@const choices = catalog.value}
			<form class="contents" onsubmit={saveDefaults}>
				<Card.Content class="grid gap-4 sm:grid-cols-2">
					{#if withdrawn}
						<Alert.Root class="sm:col-span-2">
							<TriangleAlertIcon />
							<Alert.Title>Your saved default is no longer offered</Alert.Title>
							<Alert.Description>
								New sandboxes get what is shown here instead. Save to keep it, or pick another.
							</Alert.Description>
						</Alert.Root>
					{/if}
					<Field.Field>
						<Field.Label for="default-image">Image</Field.Label>
						<NativeSelect.Root id="default-image" bind:value={picked.image} class="w-full">
							{#each choices.images as choice (choice.key)}
								<NativeSelect.Option value={choice.key}>{choice.label}</NativeSelect.Option>
							{/each}
						</NativeSelect.Root>
					</Field.Field>
					<Field.Field>
						<Field.Label for="default-size">Size</Field.Label>
						<NativeSelect.Root id="default-size" bind:value={picked.size} class="w-full">
							{#each choices.sizes as choice (choice.key)}
								<NativeSelect.Option value={choice.key}>{choice.label}</NativeSelect.Option>
							{/each}
						</NativeSelect.Root>
					</Field.Field>
				</Card.Content>
				<Card.Footer class="flex-wrap gap-3">
					<Button type="submit" disabled={saving || !(dirty || withdrawn)}>
						{#if saving}
							<Spinner />
						{/if}
						Save defaults
					</Button>
					{#if dirty}
						<Button type="button" variant="outline" disabled={saving} onclick={resetDefaults}>
							Reset
						</Button>
					{/if}
					{#if saveFailure}
						<p class="text-sm text-destructive first-letter:uppercase" role="alert">
							{saveFailure}
						</p>
					{:else}
						<p class="text-sm text-muted-foreground" role="status">
							{dirty ? 'Unsaved changes' : ''}
						</p>
					{/if}
				</Card.Footer>
			</form>
		{/if}
	</Card.Root>

	<Card.Root>
		<Card.Header>
			<Card.Title>Deleting from inside a sandbox</Card.Title>
			<Card.Description>
				With this on, running
				<code class="font-mono text-xs wrap-anywhere">corral delete --confirm &lt;name&gt;</code>
				in one of your sandboxes puts it to sleep at once and deletes it, with its home directory, 10
				minutes later. Until then you can cancel the deletion on the sandbox's page. It saves as soon
				as you switch it.
			</Card.Description>
		</Card.Header>
		<Card.Content class="grid gap-2">
			{#if insideDelete.current.state === 'failed'}
				<LoadFailed
					title="Loading the setting failed"
					error={insideDelete.current.error}
					onretry={() => insideDelete.reload()}
				/>
			{:else if !insideDelete.value}
				<Skeleton class="h-6 w-72" />
			{:else}
				<Field.Field orientation="horizontal" class="w-fit">
					<Switch
						id="inside-delete"
						checked={insideDelete.value.enabled}
						disabled={switching}
						onCheckedChange={allowInsideDelete}
					/>
					<Field.Label for="inside-delete">Allow corral delete in my sandboxes</Field.Label>
					{#if switching}
						<Spinner />
					{/if}
				</Field.Field>
				{#if switchFailure}
					<p class="text-sm text-destructive first-letter:uppercase" role="alert">
						{switchFailure}
					</p>
				{/if}
			{/if}
		</Card.Content>
	</Card.Root>

	<Card.Root class="ring-destructive/30">
		<Card.Header>
			<Card.Title>Delete your account</Card.Title>
			<Card.Description>
				Deletes your sandboxes with their home directories, your SSH keys, your environment and your
				account, for good.
			</Card.Description>
		</Card.Header>
		<Card.Footer>
			<Button variant="destructive" onclick={() => (deleting = true)}>Delete account</Button>
		</Card.Footer>
	</Card.Root>
</div>

<ConfirmDialog
	bind:open={deleting}
	title="Delete your account?"
	action="Delete account"
	confirmName={user.handle}
	onconfirm={remove}
>
	Every sandbox you own is deleted with its home directory, along with your SSH keys and
	environment. It cannot be undone.
</ConfirmDialog>
