<script lang="ts">
	import {
		changeSharedEntry,
		createSharedEntry,
		removeSharedEntry,
		sharedSetup
	} from '#lib/api/settings.js';
	import ListSkeleton from '#lib/components/list-skeleton.svelte';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import LlmUrlField from '#lib/components/presets/llm-url-field.svelte';
	import SaveForm from '#lib/components/settings/save-form.svelte';
	import * as Card from '#lib/components/ui/card/index.js';
	import {
		NAMES,
		PROVIDERS,
		readSharedLlm,
		sharedLlmSummary,
		sharedUrl,
		type Provider,
		type SharedLlm
	} from '#lib/presets/llm.js';
	import { settings } from '#lib/setup.js';
	import { SettingsForm } from '#lib/settings-form.svelte.js';

	let { onsaved }: { onsaved: () => Promise<void> } = $props();

	const form = new SettingsForm(
		async () => readSharedLlm(await sharedSetup()),
		send,
		'Saved the shared addresses'
	);

	/** Leaves each provider's address in a required shared entry of its own, or in none. */
	async function send(urls: SharedLlm) {
		const shared = await sharedSetup();
		const now = readSharedLlm(shared);
		for (const provider of PROVIDERS) {
			const held = sharedUrl(shared, provider);
			const url = urls[provider].trim();
			if (url === now[provider]) {
				continue;
			}
			if (!held) {
				await createSharedEntry({
					shared: 'required',
					kind: 'variable',
					target: NAMES[provider].url,
					secret: false,
					applyToNew: true,
					value: { text: url }
				});
			} else if (url === '') {
				await removeSharedEntry(held.id);
			} else {
				await changeSharedEntry(held.id, {
					...settings(held),
					shared: 'required',
					value: { text: url }
				});
			}
		}
		await onsaved();
	}

	const saved = $derived(form.loaded.value);
	const copiedFrom = $derived.by(() => {
		const urls = form.edited;
		return urls ? PROVIDERS.find((provider) => urls[provider].trim() !== '') : undefined;
	});

	function copy(urls: SharedLlm, from: Provider) {
		const url = urls[from].trim();
		for (const provider of PROVIDERS) {
			urls[provider] = url;
		}
	}
</script>

<Card.Root role="region" aria-labelledby="shared-llm-title">
	<Card.Header>
		<Card.Title><h2 id="shared-llm-title">Addresses for every sandbox</h2></Card.Title>
		{#if saved}
			<Card.Description class="wrap-anywhere">
				{sharedLlmSummary(saved) ?? 'No shared addresses'}
			</Card.Description>
		{/if}
	</Card.Header>
	<Card.Content>
		{#if form.loaded.current.state === 'failed'}
			<LoadFailed
				title="Loading the shared addresses failed"
				error={form.loaded.current.error}
				onretry={() => form.loaded.reload()}
			/>
		{:else if !form.edited}
			<ListSkeleton what="the shared addresses" />
		{:else}
			{@const urls = form.edited}
			<SaveForm {form} what="the shared addresses">
				<div class="grid gap-6 sm:grid-cols-2">
					{#each PROVIDERS as provider (provider)}
						<LlmUrlField
							id="shared-{provider}-url"
							label="{NAMES[provider].label} base URL"
							bind:value={urls[provider]}
							oncopy={copiedFrom === provider ? () => copy(urls, provider) : null}
						/>
					{/each}
				</div>
			</SaveForm>
		{/if}
	</Card.Content>
</Card.Root>
