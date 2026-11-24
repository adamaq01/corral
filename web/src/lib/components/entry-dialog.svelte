<script lang="ts">
	import { untrack } from 'svelte';
	import { environment } from '#lib/api/environment.js';
	import type { EntryKind, EnvironmentEntry, ValueInput } from '#lib/api/generated/index.js';
	import FormDialog from '#lib/components/form-dialog.svelte';
	import ValueField from '#lib/components/value-field.svelte';
	import * as Field from '#lib/components/ui/field/index.js';
	import { Input } from '#lib/components/ui/input/index.js';

	let {
		open = $bindable(),
		kind,
		onCreated
	}: {
		open: boolean;
		kind: EntryKind;
		onCreated: (entry: EnvironmentEntry) => Promise<void>;
	} = $props();

	let target = $state('');
	let value = $state<ValueInput | null>(null);
	let secret = $state(false);
	let template = $state(false);
	let applyToNew = $state(true);
	let fresh = $state(0);

	$effect(() => {
		if (open) {
			untrack(() => {
				target = '';
				secret = false;
				template = false;
				applyToNew = true;
				fresh += 1;
			});
		}
	});

	const variable = $derived(kind === 'variable');

	async function save() {
		if (!value) {
			return;
		}
		const created = await environment.create({
			kind,
			target: variable ? target : target.replace(/^~\//, ''),
			secret,
			template: !variable && template,
			applyToNew,
			value
		});
		await onCreated(created);
	}
</script>

<FormDialog
	bind:open
	title={variable ? 'Add a variable' : 'Add a file'}
	description={variable
		? 'Each SSH session of the sandboxes it is attached to gets it as an environment variable.'
		: 'Written into the home of the sandboxes it is attached to at each start, replacing what is there.'}
	submit={variable ? 'Save variable' : 'Save file'}
	ready={value !== null && target !== ''}
	onsubmit={save}
>
	{#if variable}
		<Field.Field>
			<Field.Label for="entry-target">Name</Field.Label>
			<Input
				id="entry-target"
				aria-describedby="entry-target-hint"
				bind:value={() => target, (typed) => (target = typed.toUpperCase())}
				required
				maxlength={64}
				autocomplete="off"
				autocapitalize="characters"
				spellcheck={false}
				class="font-mono"
				placeholder="OPENAI_API_KEY"
			/>
			<Field.Description id="entry-target-hint">
				Capital letters, digits and underscores.
			</Field.Description>
		</Field.Field>
	{:else}
		<Field.Field>
			<Field.Label for="entry-target">Path in the home</Field.Label>
			<Input
				id="entry-target"
				aria-describedby="entry-target-hint"
				bind:value={target}
				required
				maxlength={1024}
				autocomplete="off"
				autocapitalize="off"
				spellcheck={false}
				class="font-mono"
				placeholder=".config/tool/config.toml"
			/>
			<Field.Description id="entry-target-hint">
				Relative to the home directory. Missing directories are made.
			</Field.Description>
		</Field.Field>
	{/if}
	{#key fresh}
		<ValueField id="entry-value" {kind} {secret} bind:value />
	{/key}
	<fieldset class="grid gap-3">
		<legend class="sr-only">Options</legend>
		<label class="flex items-start gap-2 text-sm">
			<input type="checkbox" class="mt-0.5 size-4 accent-primary" bind:checked={secret} />
			<span>
				<span class="font-medium">Secret</span>
				<span class="block text-muted-foreground">
					Kept in the secret store and never shown again{variable
						? ''
						: ', written readable by the sandbox user alone'}.
				</span>
			</span>
		</label>
		{#if !variable}
			<label class="flex items-start gap-2 text-sm">
				<input type="checkbox" class="mt-0.5 size-4 accent-primary" bind:checked={template} />
				<span>
					<span class="font-medium">Template</span>
					<span class="block text-muted-foreground">
						Each <code class="font-mono text-xs">{'{{ NAME }}'}</code> in the text takes the value of
						the variable NAME attached to the same sandbox.
					</span>
				</span>
			</label>
		{/if}
		<label class="flex items-start gap-2 text-sm">
			<input type="checkbox" class="mt-0.5 size-4 accent-primary" bind:checked={applyToNew} />
			<span>
				<span class="font-medium">Apply to new sandboxes</span>
				<span class="block text-muted-foreground">
					Each sandbox you create from now on gets it. Attach it to existing ones on their page.
				</span>
			</span>
		</label>
	</fieldset>
</FormDialog>
