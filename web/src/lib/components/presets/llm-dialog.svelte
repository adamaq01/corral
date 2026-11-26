<script lang="ts">
	import LockIcon from '@lucide/svelte/icons/lock';
	import { tick, untrack } from 'svelte';
	import { toast } from 'svelte-sonner';
	import { ApiError, failureMessage } from '#lib/api/client.js';
	import type { SetupEntry } from '#lib/api/generated/index.js';
	import FormDialog from '#lib/components/form-dialog.svelte';
	import PasswordInput from '#lib/components/password-input.svelte';
	import LlmUrlField from '#lib/components/presets/llm-url-field.svelte';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Field from '#lib/components/ui/field/index.js';
	import { applyChanges } from '#lib/presets/apply.js';
	import {
		NAMES,
		PROVIDERS,
		fieldNamed,
		needsKey,
		readLlm,
		writeLlm,
		type Provider
	} from '#lib/presets/llm.js';

	let {
		open = $bindable(),
		entries,
		onchanged
	}: {
		open: boolean;
		entries: SetupEntry[];
		/** Called after a save, which may have changed some entries even when it failed. */
		onchanged: () => Promise<void>;
	} = $props();

	/**
	 * A provider as the member fills it in. A key typed replaces the one set, which an empty field
	 * keeps unless the row was cleared.
	 */
	type Row = { url: string; key: string; replacing: boolean; cleared: boolean };

	const KEY_PLACEHOLDERS: Record<Provider, string> = { anthropic: 'sk-ant-...', openai: 'sk-...' };

	function each<T>(value: (provider: Provider) => T): Record<Provider, T> {
		return { anthropic: value('anthropic'), openai: value('openai') };
	}

	const read = $derived(readLlm(entries));
	let rows = $state<Record<Provider, Row>>(
		each(() => ({ url: '', key: '', replacing: false, cleared: false }))
	);
	let refusal = $state<{ provider: Provider; field: 'url' | 'key'; message: string } | null>(null);

	$effect(() => {
		if (open) {
			untrack(() => {
				rows = each((provider) => ({
					url: read.rows[provider].url,
					key: '',
					replacing: false,
					cleared: false
				}));
				refusal = null;
			});
		}
	});

	const typed = (provider: Provider) => rows[provider].key.trim();
	const kept = (provider: Provider) => read.rows[provider].keySet && !rows[provider].cleared;
	const needs = $derived(
		needsKey(entries, {
			rows: each((provider) => ({
				url: rows[provider].url,
				key: kept(provider) ? null : typed(provider)
			}))
		})
	);
	const changes = $derived(
		writeLlm(entries, {
			rows: each((provider) => ({
				url: rows[provider].url,
				key: kept(provider) && typed(provider) === '' ? null : typed(provider)
			}))
		})
	);
	const ready = $derived(
		changes.length > 0 && PROVIDERS.every((provider) => !needs[provider] || typed(provider) !== '')
	);
	const copiedFrom = $derived(
		PROVIDERS.some((provider) => read.rows[provider].urlLocked)
			? undefined
			: PROVIDERS.find((provider) => rows[provider].url.trim() !== '')
	);

	const keyOpen = (provider: Provider) =>
		!kept(provider) || rows[provider].replacing || needs[provider];
	const removable = (provider: Provider) =>
		(!read.rows[provider].keyLocked && (kept(provider) || typed(provider) !== '')) ||
		(!read.rows[provider].urlLocked && rows[provider].url.trim() !== '');
	const refused = (provider: Provider, field: 'url' | 'key') =>
		refusal?.provider === provider && refusal.field === field ? refusal.message : null;

	function copy(from: Provider) {
		const url = rows[from].url.trim();
		for (const provider of PROVIDERS) {
			rows[provider].url = url;
		}
	}

	async function focus(id: string) {
		await tick();
		document.getElementById(id)?.focus();
	}

	async function replace(provider: Provider) {
		rows[provider].replacing = true;
		await focus(`llm-${provider}-key`);
	}

	async function remove(provider: Provider) {
		const locked = read.rows[provider].urlLocked;
		rows[provider] = {
			url: locked ? rows[provider].url : '',
			key: '',
			replacing: false,
			cleared: true
		};
		await focus(locked ? `llm-${provider}-key` : `llm-${provider}-url`);
	}

	async function save(): Promise<void | false> {
		refusal = null;
		try {
			await applyChanges(changes);
		} catch (failure) {
			const named = failure instanceof ApiError ? fieldNamed(failure.message) : null;
			if (named === null) {
				throw failure;
			}
			refusal = { ...named, message: failureMessage(failure) };
			return false;
		} finally {
			await onchanged();
		}
		toast.success('Saved the LLM providers');
	}
</script>

<FormDialog
	bind:open
	title="LLM providers"
	description="The keys and addresses your agents use: Claude Code, Codex, Pi and oh-my-pi read them from the sandbox's variables and their own files."
	submit="Save providers"
	{ready}
	onsubmit={save}
>
	{#each PROVIDERS as provider (provider)}
		{@const label = NAMES[provider].label}
		{@const keyId = `llm-${provider}-key`}
		{@const keyRefused = refused(provider, 'key')}
		<Card.Root size="sm">
			<Card.Content>
				<Field.Set>
					<Field.Legend>{label}</Field.Legend>
					<LlmUrlField
						id="llm-{provider}-url"
						label="Base URL"
						bind:value={rows[provider].url}
						locked={read.rows[provider].urlLocked}
						error={refused(provider, 'url')}
						oncopy={copiedFrom === provider ? () => copy(provider) : null}
					/>
					<Field.Field data-invalid={keyRefused ? true : undefined}>
						{#if read.rows[provider].keyLocked}
							<div class="flex flex-wrap items-center gap-2">
								<Field.Title>Key</Field.Title>
								<Badge variant="outline">
									<LockIcon aria-hidden="true" />
									Set by the admin
								</Badge>
							</div>
							<div class="flex h-8 items-center rounded-lg border px-2.5">
								<span class="text-sm text-muted-foreground">Set, hidden</span>
							</div>
						{:else if keyOpen(provider)}
							<Field.Label for={keyId}>Key</Field.Label>
							<PasswordInput
								id={keyId}
								bind:value={rows[provider].key}
								placeholder={KEY_PLACEHOLDERS[provider]}
								autocomplete="off"
								spellcheck={false}
								aria-invalid={keyRefused ? true : undefined}
								aria-describedby={keyRefused || needs[provider] ? `${keyId}-hint` : undefined}
							/>
							{#if keyRefused}
								<Field.Error id="{keyId}-hint" class="first-letter:uppercase">
									{keyRefused}
								</Field.Error>
							{:else if needs[provider]}
								<Field.Description id="{keyId}-hint">
									Type the key again so it is also sent as a bearer token.
								</Field.Description>
							{/if}
						{:else}
							<Field.Title>Key</Field.Title>
							<div class="flex h-8 items-center justify-between gap-2 rounded-lg border px-2.5">
								<span class="text-sm text-muted-foreground">Set, hidden</span>
								<Button
									variant="link"
									size="sm"
									class="h-auto p-0"
									onclick={() => replace(provider)}
								>
									Replace<span class="sr-only"> the {label} key</span>
								</Button>
							</div>
						{/if}
					</Field.Field>
					{#if removable(provider)}
						<Button
							variant="link"
							size="sm"
							class="h-auto w-fit p-0"
							onclick={() => remove(provider)}
						>
							Remove<span class="sr-only"> {label}</span>
						</Button>
					{/if}
				</Field.Set>
			</Card.Content>
		</Card.Root>
	{/each}
</FormDialog>
