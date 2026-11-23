<script lang="ts">
	import { goto } from '$app/navigation';
	import { failureMessage } from '#lib/api/client.js';
	import type { SandboxDefaults } from '#lib/api/generated/index.js';
	import { deleteAccount, setSandboxDefaults } from '#lib/api/me.js';
	import { sandboxes } from '#lib/api/sandboxes.js';
	import ConfirmDialog from '#lib/components/confirm-dialog.svelte';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import PageHeader from '#lib/components/page-header.svelte';
	import UserAvatar from '#lib/components/user-avatar.svelte';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Field from '#lib/components/ui/field/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import * as NativeSelect from '#lib/components/ui/native-select/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { Spinner } from '#lib/components/ui/spinner/index.js';
	import { Remote } from '#lib/remote.svelte.js';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
	const user = $derived(data.user);

	const roles = { member: 'Member', admin: 'Admin' } as const;

	let picked = $state<SandboxDefaults | null>(null);
	const catalog = new Remote(async () => {
		const loaded = await sandboxes.catalog();
		picked = { image: loaded.defaultImage, size: loaded.defaultSize };
		return loaded;
	});
	let saving = $state(false);
	let saved = $state<{ ok: boolean; message: string } | null>(null);

	async function saveDefaults(event: SubmitEvent) {
		event.preventDefault();
		if (!picked) {
			return;
		}
		saving = true;
		saved = null;
		try {
			await setSandboxDefaults(picked);
			saved = { ok: true, message: 'Saved. New sandboxes start from these.' };
		} catch (failure) {
			saved = { ok: false, message: failureMessage(failure) };
		} finally {
			saving = false;
		}
	}

	let deleting = $state(false);
	let typed = $state('');

	async function remove() {
		await deleteAccount();
		await goto('/sign-in', { replace: true, invalidateAll: true });
	}
</script>

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
					<Button type="submit" disabled={saving}>
						{#if saving}
							<Spinner />
						{/if}
						Save defaults
					</Button>
					{#if saved}
						<p
							class={[
								'text-sm first-letter:uppercase',
								saved.ok ? 'text-muted-foreground' : 'text-destructive'
							]}
							role={saved.ok ? 'status' : 'alert'}
						>
							{saved.message}
						</p>
					{/if}
				</Card.Footer>
			</form>
		{/if}
	</Card.Root>

	<Card.Root class="ring-destructive/30">
		<Card.Header>
			<Card.Title>Delete your account</Card.Title>
			<Card.Description>
				Deletes your sandboxes with their home directories, your SSH keys, your credentials and your
				account, for good.
			</Card.Description>
		</Card.Header>
		<Card.Footer>
			<Button
				variant="destructive"
				onclick={() => {
					typed = '';
					deleting = true;
				}}
			>
				Delete account
			</Button>
		</Card.Footer>
	</Card.Root>
</div>

<ConfirmDialog
	bind:open={deleting}
	title="Delete your account?"
	action="Delete account"
	ready={typed === user.handle}
	onconfirm={remove}
>
	Every sandbox you own is deleted with its home directory, along with your SSH keys and
	credentials. It cannot be undone.
	{#snippet fields()}
		<div class="grid gap-2">
			<Label for="confirm-handle">
				Type <span class="font-mono">{user.handle}</span> to confirm
			</Label>
			<Input
				id="confirm-handle"
				bind:value={typed}
				autocomplete="off"
				autocapitalize="none"
				spellcheck={false}
			/>
		</div>
	{/snippet}
</ConfirmDialog>
