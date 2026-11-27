<script lang="ts">
	import { tick, untrack } from 'svelte';
	import { toast } from 'svelte-sonner';
	import { ApiError, failureMessage } from '#lib/api/client.js';
	import type { SetupEntry, ValueInput } from '#lib/api/generated/index.js';
	import FormDialog from '#lib/components/form-dialog.svelte';
	import PasswordInput from '#lib/components/password-input.svelte';
	import ValueField from '#lib/components/value-field.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Field from '#lib/components/ui/field/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import * as RadioGroup from '#lib/components/ui/radio-group/index.js';
	import { applyChanges } from '#lib/presets/apply.js';
	import { namesToken, readGit, writeGit, type GitForm, type GitState } from '#lib/presets/git.js';

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

	type Reach = GitState['reach']['by'];

	/**
	 * The identity as the member fills it in. A token or key typed replaces the one set, which an
	 * empty field keeps.
	 */
	type Form = {
		name: string;
		email: string;
		by: Reach;
		host: string;
		user: string;
		token: string;
		privateKey: ValueInput | null;
		publicKey: ValueInput | null;
		replacing: boolean;
	};

	const CHOICES: { by: Reach; label: string; hint: string }[] = [
		{ by: 'token', label: 'A token', hint: 'For HTTPS, such as a personal access token.' },
		{
			by: 'ssh',
			label: 'An SSH key',
			hint: 'Written to ~/.ssh/id_ed25519, replacing a key kept there.'
		},
		{
			by: 'nothing',
			label: 'Nothing',
			hint: 'Only the name and email; a token or SSH key set here is removed.'
		}
	];

	function fresh({ name, email, reach }: GitState): Form {
		return {
			name,
			email,
			by: reach.by,
			host: reach.by === 'token' ? reach.host : '',
			user: reach.by === 'token' ? reach.user : '',
			token: '',
			privateKey: null,
			publicKey: null,
			replacing: false
		};
	}

	const read = $derived(readGit(entries));
	let form = $state(fresh(readGit([])));
	let refusal = $state<string | null>(null);

	$effect(() => {
		if (open) {
			untrack(() => {
				form = fresh(read);
				refusal = null;
			});
		}
	});

	const decoder = new TextDecoder();

	function keyText(value: ValueInput | null): string {
		if (value === null) {
			return '';
		}
		return 'text' in value
			? value.text
			: decoder.decode(Uint8Array.from(atob(value.base64), (char) => char.charCodeAt(0)));
	}

	const host = $derived(form.host.trim());
	const user = $derived(form.user.trim());
	const token = $derived(form.token.trim());
	const privateKey = $derived(keyText(form.privateKey));
	const publicKey = $derived(keyText(form.publicKey).trim());
	const hostProblem = $derived(
		/[\s/@]/.test(host) ? 'Only the host, such as github.com, without https:// or a path.' : null
	);
	const kept = $derived(form.by !== 'nothing' && form.by === read.reach.by);
	const moved = $derived(
		read.reach.by === 'token' &&
			form.by === 'token' &&
			(host !== read.reach.host || user !== read.reach.user)
	);
	const secretOpen = $derived(!kept || form.replacing || moved);

	const reach = $derived<GitForm['reach']>(
		form.by === 'token'
			? { by: 'token', token: token ? { host, user, token } : null }
			: form.by === 'ssh'
				? { by: 'ssh', key: privateKey.trim() ? { privateKey, publicKey } : null }
				: { by: 'nothing' }
	);
	const changes = $derived(
		writeGit(entries, { name: form.name.trim(), email: form.email.trim(), reach })
	);
	const complete = $derived(
		form.by === 'token'
			? host !== '' && user !== '' && !hostProblem && (token !== '' || (kept && !moved))
			: form.by === 'ssh'
				? privateKey.trim() !== '' || (kept && publicKey === '')
				: true
	);
	const ready = $derived(changes.length > 0 && complete);

	async function replace() {
		form.replacing = true;
		await tick();
		document.getElementById(form.by === 'token' ? 'git-token' : 'git-private-key')?.focus();
	}

	async function save(): Promise<void | false> {
		refusal = null;
		try {
			await applyChanges(changes);
		} catch (failure) {
			if (!(failure instanceof ApiError && namesToken(failure.message))) {
				throw failure;
			}
			refusal = failureMessage(failure);
			return false;
		} finally {
			await onchanged();
		}
		toast.success('Saved your git identity');
	}
</script>

{#snippet keptSecret(title: string, what: string)}
	<Field.Field>
		<Field.Title>{title}</Field.Title>
		<div class="flex h-8 items-center justify-between gap-2 rounded-lg border px-2.5">
			<span class="text-sm text-muted-foreground">Set, hidden</span>
			<Button variant="link" size="sm" class="h-auto p-0" onclick={replace}>
				Replace<span class="sr-only"> {what}</span>
			</Button>
		</div>
	</Field.Field>
{/snippet}

<FormDialog
	bind:open
	title="Git identity"
	description="Written as ~/.gitconfig, with the token or SSH key git uses to reach your repositories."
	submit="Save identity"
	{ready}
	onsubmit={save}
>
	<Field.Field>
		<Field.Label for="git-name">Name</Field.Label>
		<Input id="git-name" bind:value={form.name} autocomplete="name" />
	</Field.Field>
	<Field.Field>
		<Field.Label for="git-email">Email</Field.Label>
		<Input
			id="git-email"
			type="email"
			bind:value={form.email}
			autocomplete="email"
			autocapitalize="none"
			spellcheck={false}
		/>
	</Field.Field>
	<Field.Set>
		<Field.Legend variant="label">Reach repositories with</Field.Legend>
		<RadioGroup.Root bind:value={form.by} class="grid gap-3">
			{#each CHOICES as choice (choice.by)}
				<Field.Field orientation="horizontal">
					<RadioGroup.Item
						value={choice.by}
						id="git-reach-{choice.by}"
						aria-describedby="git-reach-{choice.by}-hint"
					/>
					<Field.Content>
						<Field.Label for="git-reach-{choice.by}">{choice.label}</Field.Label>
						<Field.Description id="git-reach-{choice.by}-hint">{choice.hint}</Field.Description>
					</Field.Content>
				</Field.Field>
			{/each}
		</RadioGroup.Root>
	</Field.Set>
	{#if form.by === 'token'}
		<div class="grid gap-4 sm:grid-cols-2">
			<Field.Field data-invalid={hostProblem ? true : undefined}>
				<Field.Label for="git-host">Host</Field.Label>
				<Input
					id="git-host"
					bind:value={form.host}
					placeholder="github.com"
					autocomplete="off"
					autocapitalize="none"
					spellcheck={false}
					aria-invalid={hostProblem ? true : undefined}
					aria-describedby={hostProblem ? 'git-host-hint' : undefined}
				/>
				{#if hostProblem}
					<Field.Error id="git-host-hint">{hostProblem}</Field.Error>
				{/if}
			</Field.Field>
			<Field.Field>
				<Field.Label for="git-user">User</Field.Label>
				<Input
					id="git-user"
					bind:value={form.user}
					placeholder="git"
					autocomplete="off"
					autocapitalize="none"
					spellcheck={false}
				/>
			</Field.Field>
		</div>
		{#if secretOpen}
			<Field.Field data-invalid={refusal ? true : undefined}>
				<Field.Label for="git-token">Token</Field.Label>
				<PasswordInput
					id="git-token"
					bind:value={form.token}
					autocomplete="off"
					spellcheck={false}
					aria-invalid={refusal ? true : undefined}
					aria-describedby={refusal || moved ? 'git-token-hint' : undefined}
				/>
				{#if refusal}
					<Field.Error id="git-token-hint" class="first-letter:uppercase">{refusal}</Field.Error>
				{:else if moved}
					<Field.Description id="git-token-hint">
						Type the token again to save it for this host and user.
					</Field.Description>
				{/if}
			</Field.Field>
		{:else}
			{@render keptSecret('Token', 'the token')}
		{/if}
	{:else if form.by === 'ssh'}
		{#if secretOpen}
			<ValueField
				id="git-private-key"
				kind="file"
				secret
				label="Private key"
				description="Paste the key, or upload its file."
				bind:value={form.privateKey}
			/>
			<ValueField
				id="git-public-key"
				kind="file"
				secret={false}
				label="Public key"
				description="Optional. Paste the key, or upload its .pub file."
				bind:value={form.publicKey}
			/>
		{:else}
			{@render keptSecret('Private key', 'the SSH key')}
		{/if}
	{/if}
</FormDialog>
