<script lang="ts">
	import FileTextIcon from '@lucide/svelte/icons/file-text';
	import LockIcon from '@lucide/svelte/icons/lock';
	import VariableIcon from '@lucide/svelte/icons/variable';
	import { untrack } from 'svelte';
	import type { Catalog, Sandbox, SetupEntry } from '#lib/api/generated/index.js';
	import { sandboxes } from '#lib/api/sandboxes.js';
	import { bytesLabel } from '#lib/catalog.js';
	import FormDialog from '#lib/components/form-dialog.svelte';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Checkbox } from '#lib/components/ui/checkbox/index.js';
	import * as Field from '#lib/components/ui/field/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import * as NativeSelect from '#lib/components/ui/native-select/index.js';
	import { BRANCH_PATTERN, REPOSITORY_PATTERN, cloneDirectory } from '#lib/sandbox.js';
	import { SHARING, chosenSetup, newSandboxSetup, replacement, where } from '#lib/setup.js';

	let {
		open = $bindable(),
		catalog,
		handle,
		count,
		setup,
		onCreated
	}: {
		open: boolean;
		catalog: Catalog;
		/** The signed-in user's handle, the second half of each sandbox's SSH username. */
		handle: string;
		/** How many sandboxes the user has now. */
		count: number;
		/** The user's setup entries, shared and their own. */
		setup: SetupEntry[];
		onCreated: (sandbox: Sandbox) => Promise<void>;
	} = $props();

	// Never a catalog key, which holds only lowercase letters, digits and dashes.
	const NAMED = '+named';

	let name = $state('');
	let image = $state('');
	let reference = $state('');
	let size = $state('');
	let comesWith = $state(newSandboxSetup([]));
	let url = $state('');
	let branch = $state('');

	$effect(() => {
		if (open) {
			untrack(() => {
				name = count === 0 ? 'main' : '';
				image = catalog.defaultImage;
				reference = '';
				size = catalog.defaultSize;
				comesWith = newSandboxSetup(setup);
				url = '';
				branch = '';
			});
		}
	});

	const full = $derived(count >= catalog.maxSandboxes);

	const named = $derived(catalog.namedImages && image === NAMED ? catalog.namedImages : undefined);

	// A `pattern` attribute compiles with the `v` flag, where a class needs its `-` escaped.
	const NAME_PATTERN = /^[a-z0-9\-]{1,32}$/;
	const valid = $derived(NAME_PATTERN.test(name));
	const invalid = $derived(name !== '' && !valid);
	const preview = $derived(
		`ssh -p ${catalog.ssh.port} ${valid ? name : '<name>'}.${handle}@${catalog.ssh.host}`
	);

	const REPOSITORY_RULE = 'An https://, ssh:// or git@host:path address, with no password in it.';
	const repositoryUrl = $derived(url.trim());
	const urlValid = $derived(REPOSITORY_PATTERN.test(repositoryUrl));
	const repositoryInvalid = $derived(repositoryUrl !== '' && !urlValid);
	const branchName = $derived(branch.trim());
	const branchInvalid = $derived(
		repositoryUrl !== '' && branchName !== '' && !BRANCH_PATTERN.test(branchName)
	);

	async function create() {
		const choice = named ? { imageReference: reference.trim() } : { image };
		await onCreated(
			await sandboxes.create({
				name,
				...choice,
				size,
				setup: chosenSetup(comesWith),
				repository: repositoryUrl
					? { url: repositoryUrl, branch: branchName || undefined }
					: undefined
			})
		);
	}
</script>

<FormDialog
	bind:open
	title="New sandbox"
	description={full
		? `You have ${count} of ${catalog.maxSandboxes} sandboxes, the most you can have. Delete one to make room.`
		: `A sandbox with its own home directory. You have ${count} of ${catalog.maxSandboxes} sandboxes.`}
	submit="Create sandbox"
	pendingLabel={named ? 'Checking the image…' : 'Creating…'}
	ready={!full && valid && !repositoryInvalid && !branchInvalid}
	onsubmit={create}
>
	<Field.Field data-invalid={invalid ? true : undefined}>
		<Field.Label for="sandbox-name">Name</Field.Label>
		<Input
			id="sandbox-name"
			aria-describedby="sandbox-name-hint"
			aria-invalid={invalid ? true : undefined}
			bind:value={name}
			required
			maxlength={32}
			pattern={NAME_PATTERN.source.slice(1, -1)}
			autocomplete="off"
			autocapitalize="none"
			spellcheck={false}
			disabled={full}
		/>
		{#if invalid}
			<Field.Error id="sandbox-name-hint">
				Up to 32 lowercase letters, digits and dashes.
			</Field.Error>
		{:else}
			<Field.Description id="sandbox-name-hint">
				Up to 32 lowercase letters, digits and dashes. You then connect with
				<code class="font-mono text-xs wrap-anywhere">{preview}</code>.
			</Field.Description>
		{/if}
	</Field.Field>
	<Field.Field>
		<Field.Label for="sandbox-image">Image</Field.Label>
		<NativeSelect.Root id="sandbox-image" bind:value={image} class="w-full" disabled={full}>
			{#each catalog.images as choice (choice.key)}
				<NativeSelect.Option value={choice.key}>{choice.label}</NativeSelect.Option>
			{/each}
			{#if catalog.namedImages}
				<NativeSelect.Option value={NAMED}>Another image</NativeSelect.Option>
			{/if}
		</NativeSelect.Root>
	</Field.Field>
	{#if named}
		<Field.Field>
			<Field.Label for="sandbox-image-reference">Image reference</Field.Label>
			<Input
				id="sandbox-image-reference"
				aria-describedby="sandbox-image-reference-hint"
				bind:value={reference}
				required
				placeholder="debian:13"
				autocomplete="off"
				autocapitalize="none"
				spellcheck={false}
				disabled={full}
			/>
			<Field.Description id="sandbox-image-reference-hint">
				Any Linux image from {named.registries.join(', ')}, at most {bytesLabel(named.maxBytes)}
				to download. Corral brings the sandbox user, SSH and Herdr to it.
			</Field.Description>
		</Field.Field>
	{/if}
	<Field.Field>
		<Field.Label for="sandbox-size">Size</Field.Label>
		<NativeSelect.Root id="sandbox-size" bind:value={size} class="w-full" disabled={full}>
			{#each catalog.sizes as choice (choice.key)}
				<NativeSelect.Option value={choice.key}>{choice.label}</NativeSelect.Option>
			{/each}
		</NativeSelect.Root>
	</Field.Field>
	<Field.Set class="gap-3">
		<Field.Legend>Setup</Field.Legend>
		{#if comesWith.required.length + comesWith.choices.length === 0}
			<Field.Description>
				No variables or files yet. Add an API key, a token or a config file on
				<a href="/setup">Setup</a> first.
			</Field.Description>
		{:else}
			<Field.Description>What it comes with, from <a href="/setup">Setup</a>.</Field.Description>
			<ul class="grid gap-1" aria-label="What it comes with">
				{#each comesWith.required as entry (entry.id)}
					<li class="flex items-start gap-3 rounded-md border bg-muted/50 px-3 py-2.5">
						<LockIcon class="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
						{@render described(entry)}
					</li>
				{/each}
				{#each comesWith.choices as row (row.entry.id)}
					{@const own = replacement(comesWith, row)}
					{@const note = `sandbox-setup-replaced-${row.entry.id}`}
					<li>
						<label
							class="flex cursor-pointer items-start gap-3 rounded-md border px-3 py-2.5 has-disabled:cursor-not-allowed has-data-checked:border-primary/50 has-data-checked:bg-primary/5"
						>
							<Checkbox
								class="mt-0.5"
								bind:checked={() => row.chosen && !own, (checked) => (row.chosen = checked)}
								disabled={full || own !== undefined}
								aria-describedby={own ? note : undefined}
							/>
							{@render described(row.entry)}
						</label>
						{#if own}
							<Field.Description id={note} class="px-3 pt-1">
								Your {where(own)} takes the place of the shared one.
							</Field.Description>
						{/if}
					</li>
				{/each}
			</ul>
		{/if}
	</Field.Set>
	<Field.Set class="gap-3">
		<Field.Legend>Repository</Field.Legend>
		<Field.Description id="sandbox-repository-hint">
			Cloned into <code class="font-mono text-xs break-all">
				~/{urlValid ? cloneDirectory(repositoryUrl) : '<name>'}
			</code> at the first start, with your git token or SSH key from Setup.
		</Field.Description>
		<Field.Field data-invalid={repositoryInvalid ? true : undefined}>
			<Field.Label for="sandbox-repository">Clone URL</Field.Label>
			<Input
				id="sandbox-repository"
				aria-describedby="sandbox-repository-rule sandbox-repository-hint"
				aria-invalid={repositoryInvalid ? true : undefined}
				bind:value={url}
				placeholder="https://github.com/you/project.git"
				autocomplete="off"
				autocapitalize="none"
				spellcheck={false}
				disabled={full}
			/>
			{#if repositoryInvalid}
				<Field.Error id="sandbox-repository-rule">{REPOSITORY_RULE}</Field.Error>
			{:else}
				<Field.Description id="sandbox-repository-rule">{REPOSITORY_RULE}</Field.Description>
			{/if}
		</Field.Field>
		<Field.Field data-invalid={branchInvalid ? true : undefined}>
			<Field.Label for="sandbox-branch">Branch</Field.Label>
			<Input
				id="sandbox-branch"
				aria-describedby={branchInvalid ? 'sandbox-branch-rule' : undefined}
				aria-invalid={branchInvalid ? true : undefined}
				bind:value={branch}
				placeholder="the default branch"
				autocomplete="off"
				autocapitalize="none"
				spellcheck={false}
				disabled={full || repositoryUrl === ''}
			/>
			{#if branchInvalid}
				<Field.Error id="sandbox-branch-rule">
					A git branch name, with no spaces, .., ~, ^, :, ?, *, [ or \ in it, and no - first.
				</Field.Error>
			{/if}
		</Field.Field>
	</Field.Set>
</FormDialog>

{#snippet described(entry: SetupEntry)}
	{@const Icon = entry.kind === 'variable' ? VariableIcon : FileTextIcon}
	<span class="flex min-w-0 flex-1 flex-wrap items-center gap-1.5 text-sm font-medium">
		<Icon class="size-3.5 text-muted-foreground" aria-hidden="true" />
		<span class="font-mono break-all">{where(entry)}</span>
		{#if entry.shared}
			<Badge variant="outline">{SHARING[entry.shared].badge}</Badge>
		{/if}
		{#if entry.secret}
			<Badge variant="secondary">Secret</Badge>
		{/if}
		{#if entry.template}
			<Badge variant="outline">Template</Badge>
		{/if}
	</span>
{/snippet}
