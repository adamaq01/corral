<script lang="ts">
	import PlusIcon from '@lucide/svelte/icons/plus';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import * as settings from '#lib/api/settings.js';
	import ConfirmDialog from '#lib/components/confirm-dialog.svelte';
	import ListSkeleton from '#lib/components/list-skeleton.svelte';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import CatalogTable from '#lib/components/settings/catalog-table.svelte';
	import ConfigBadge from '#lib/components/settings/config-badge.svelte';
	import ImageDialog from '#lib/components/settings/image-dialog.svelte';
	import SaveForm from '#lib/components/settings/save-form.svelte';
	import SizeDialog from '#lib/components/settings/size-dialog.svelte';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Field from '#lib/components/ui/field/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Switch } from '#lib/components/ui/switch/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';
	import { SettingsForm } from '#lib/settings-form.svelte.js';
	import {
		BYTES_PATTERN,
		catalogChange,
		catalogView,
		put,
		refusal,
		without,
		type ImageEntry,
		type SizeEntry
	} from '#lib/settings.js';

	const form = new SettingsForm(
		async () => catalogView(await settings.catalog()),
		(view) => settings.setCatalog(catalogChange(view)),
		'Saved the catalog'
	);

	type Shown =
		| { dialog: 'image'; image: ImageEntry | null }
		| { dialog: 'size'; size: SizeEntry | null }
		| { dialog: 'remove'; kind: 'image' | 'size'; key: string };

	/** The dialog shown, kept while it closes. */
	let shown = $state<Shown | null>(null);
	let open = $state(false);

	function show(dialog: Shown) {
		shown = dialog;
		open = true;
	}
</script>

{#if form.loaded.current.state === 'failed'}
	<LoadFailed
		title="Loading the catalog failed"
		error={form.loaded.current.error}
		onretry={() => form.loaded.reload()}
	/>
{:else if !form.edited}
	<ListSkeleton what="the catalog" />
{:else}
	{@const view = form.edited}
	{@const named = view.namedImages}
	<SaveForm {form} what="the catalog">
		{#if view.defaultsFellBack}
			<Alert.Root>
				<TriangleAlertIcon />
				<Alert.Title>A saved default is no longer offered</Alert.Title>
				<Alert.Description>
					New sandboxes get the default marked below instead, the first image or size offered.
				</Alert.Description>
			</Alert.Root>
		{/if}

		<Card.Root>
			<Card.Header>
				<Card.Title><h2>Images</h2></Card.Title>
				<Card.Description>
					What members start sandboxes from. The default is what a member gets until they choose
					their own.
				</Card.Description>
				<Card.Action>
					<Button variant="outline" onclick={() => show({ dialog: 'image', image: null })}>
						<PlusIcon />Add image
					</Button>
				</Card.Action>
			</Card.Header>
			<Card.Content>
				<CatalogTable
					kind="image"
					bind:rows={view.images}
					bind:selected={view.defaults.value.image}
					selectedLocked={view.defaults.locked}
					onedit={(image) => show({ dialog: 'image', image })}
					onremove={(key) => show({ dialog: 'remove', kind: 'image', key })}
				>
					{#snippet detail(image)}
						<span class="font-mono text-xs break-all">{image.reference}</span>
					{/snippet}
				</CatalogTable>
			</Card.Content>
		</Card.Root>

		<Card.Root>
			<Card.Header>
				<Card.Title><h2>Sizes</h2></Card.Title>
				<Card.Description>
					The CPU and memory a sandbox is sure of and may use at most, and the size of its home
					directory. The default is what a member gets until they choose their own.
				</Card.Description>
				<Card.Action>
					<Button variant="outline" onclick={() => show({ dialog: 'size', size: null })}>
						<PlusIcon />Add size
					</Button>
				</Card.Action>
			</Card.Header>
			<Card.Content>
				<CatalogTable
					kind="size"
					bind:rows={view.sizes}
					bind:selected={view.defaults.value.size}
					selectedLocked={view.defaults.locked}
					onedit={(size) => show({ dialog: 'size', size })}
					onremove={(key) => show({ dialog: 'remove', kind: 'size', key })}
				>
					{#snippet detail(size)}
						CPU {size.cpuRequest} to {size.cpuLimit}, memory {size.memoryRequest} to
						{size.memoryLimit}, home {size.homeSize}
					{/snippet}
				</CatalogTable>
			</Card.Content>
		</Card.Root>

		<Card.Root>
			<Card.Header>
				<Card.Title><h2>Named images</h2></Card.Title>
				<Card.Description>
					Images members name themselves when they create a sandbox, beside the catalog's.
				</Card.Description>
				{#if named.locked}
					<Card.Action><ConfigBadge state="locked" /></Card.Action>
				{/if}
			</Card.Header>
			<Card.Content class="grid gap-6">
				<Field.Field orientation="horizontal" class="w-fit">
					<Switch id="named-images" disabled={named.locked} bind:checked={named.value.allowed} />
					<Field.Label for="named-images">Members may name their own images</Field.Label>
				</Field.Field>
				{#if named.value.allowed}
					<div class="grid gap-6 sm:grid-cols-2">
						<Field.Field>
							<Field.Label for="named-images-registries">Registries</Field.Label>
							<Textarea
								id="named-images-registries"
								autocapitalize="none"
								spellcheck={false}
								required
								disabled={named.locked}
								class="font-mono"
								aria-describedby="named-images-registries-hint"
								bind:value={named.value.registries}
							/>
							<Field.Description id="named-images-registries-hint">
								One per line: a registry such as ghcr.io, or a path in one such as ghcr.io/acme.
							</Field.Description>
						</Field.Field>
						<Field.Field>
							<Field.Label for="named-images-max-size">Largest image</Field.Label>
							<Input
								id="named-images-max-size"
								autocomplete="off"
								autocapitalize="none"
								spellcheck={false}
								pattern={BYTES_PATTERN}
								required
								disabled={named.locked}
								aria-describedby="named-images-max-size-hint"
								bind:value={named.value.maxSize}
							/>
							<Field.Description id="named-images-max-size-hint">
								The most an image may take to download, compressed. As in Kubernetes: 4Gi, 512Mi or
								1G.
							</Field.Description>
						</Field.Field>
					</div>
				{/if}
			</Card.Content>
		</Card.Root>
	</SaveForm>

	{#if shown?.dialog === 'image'}
		<ImageDialog
			bind:open
			image={shown.image}
			refuse={(key) => refusal(view.images, key, 'an image')}
			onsave={(image) => (view.images = put(view.images, image.key, image))}
		/>
	{:else if shown?.dialog === 'size'}
		<SizeDialog
			bind:open
			size={shown.size}
			refuse={(key) => refusal(view.sizes, key, 'a size')}
			onsave={(size) => (view.sizes = put(view.sizes, size.key, size))}
		/>
	{:else if shown?.dialog === 'remove'}
		{@const { kind, key } = shown}
		<ConfirmDialog
			bind:open
			title="Remove {key}?"
			action="Remove {kind}"
			confirmName={key}
			onconfirm={async () => {
				if (kind === 'image') {
					view.images = without(view.images, key);
				} else {
					view.sizes = without(view.sizes, key);
				}
			}}
		>
			Saving the catalog then removes the {kind}
			{key} for good. A {kind} that sandboxes were made from cannot be removed: disable it instead, and
			they keep it.
		</ConfirmDialog>
	{/if}
{/if}
