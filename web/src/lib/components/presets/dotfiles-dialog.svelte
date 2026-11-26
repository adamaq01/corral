<script lang="ts">
	import { tick, untrack } from 'svelte';
	import { toast } from 'svelte-sonner';
	import type { SetupEntry } from '#lib/api/generated/index.js';
	import FormDialog from '#lib/components/form-dialog.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Field from '#lib/components/ui/field/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { applyChanges } from '#lib/presets/apply.js';
	import { readDotfiles, writeDotfiles } from '#lib/presets/dotfiles.js';
	import {
		BRANCH_PATTERN,
		BRANCH_RULE,
		REPOSITORY_PATTERN,
		REPOSITORY_RULE
	} from '#lib/sandbox.js';

	let {
		open = $bindable(),
		entries,
		onchanged,
		onaddfile
	}: {
		open: boolean;
		entries: SetupEntry[];
		/** Called after a save, which may have changed some entries even when it failed. */
		onchanged: () => Promise<void>;
		onaddfile: () => void;
	} = $props();

	const read = $derived(readDotfiles(entries));
	let form = $state(readDotfiles([]));

	$effect(() => {
		if (open) {
			untrack(() => {
				form = { ...read };
			});
		}
	});

	const url = $derived(form.url.trim());
	const branch = $derived(form.branch.trim());
	const urlInvalid = $derived(url !== '' && !REPOSITORY_PATTERN.test(url));
	const branchInvalid = $derived(url !== '' && branch !== '' && !BRANCH_PATTERN.test(branch));
	const changes = $derived(writeDotfiles(entries, form));
	const ready = $derived(changes.length > 0 && !urlInvalid && !branchInvalid);

	async function remove() {
		form = { url: '', branch: '' };
		await tick();
		document.getElementById('dotfiles-repository')?.focus();
	}

	function addFile() {
		open = false;
		onaddfile();
	}

	async function save() {
		try {
			await applyChanges(changes);
		} finally {
			await onchanged();
		}
		toast.success('Saved your dotfiles');
	}
</script>

<FormDialog
	bind:open
	title="Dotfiles"
	description="Checked out into the home at a sandbox's first start, or files you add one by one."
	submit="Save dotfiles"
	{ready}
	onsubmit={save}
>
	<Field.Field data-invalid={urlInvalid ? true : undefined}>
		<Field.Label for="dotfiles-repository">Repository</Field.Label>
		<Input
			id="dotfiles-repository"
			aria-describedby="dotfiles-repository-rule"
			aria-invalid={urlInvalid ? true : undefined}
			bind:value={form.url}
			placeholder="https://github.com/you/dotfiles.git"
			autocomplete="off"
			autocapitalize="none"
			spellcheck={false}
		/>
		{#if urlInvalid}
			<Field.Error id="dotfiles-repository-rule">{REPOSITORY_RULE}</Field.Error>
		{:else}
			<Field.Description id="dotfiles-repository-rule">{REPOSITORY_RULE}</Field.Description>
		{/if}
	</Field.Field>
	<Field.Field data-invalid={branchInvalid ? true : undefined}>
		<Field.Label for="dotfiles-branch">Branch</Field.Label>
		<Input
			id="dotfiles-branch"
			aria-describedby={branchInvalid ? 'dotfiles-branch-rule' : undefined}
			aria-invalid={branchInvalid ? true : undefined}
			bind:value={form.branch}
			placeholder="the default branch"
			autocomplete="off"
			autocapitalize="none"
			spellcheck={false}
			disabled={url === ''}
		/>
		{#if branchInvalid}
			<Field.Error id="dotfiles-branch-rule">{BRANCH_RULE}</Field.Error>
		{/if}
	</Field.Field>
	{#if form.url !== '' || form.branch !== ''}
		<Button variant="link" size="sm" class="h-auto w-fit p-0" onclick={remove}>
			Remove<span class="sr-only"> the repository</span>
		</Button>
	{/if}
	<Button variant="link" size="sm" class="h-auto w-fit p-0" onclick={addFile}>
		Add a file instead
	</Button>
</FormDialog>
