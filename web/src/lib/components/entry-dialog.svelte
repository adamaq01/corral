<script lang="ts">
	import { untrack } from 'svelte';
	import { environment } from '#lib/api/environment.js';
	import type { EntryKind, EnvironmentEntry, ValueInput } from '#lib/api/generated/index.js';
	import FormDialog from '#lib/components/form-dialog.svelte';
	import ValueField from '#lib/components/value-field.svelte';
	import * as Field from '#lib/components/ui/field/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import * as RadioGroup from '#lib/components/ui/radio-group/index.js';
	import { Switch } from '#lib/components/ui/switch/index.js';
	import { homePath, nameProblem, noun, pathProblem, where } from '#lib/environment.js';

	type Settings = {
		kind: EntryKind;
		target: string;
		secret: boolean;
		template: boolean;
		executable: boolean;
		applyToNew: boolean;
	};

	let {
		open = $bindable(),
		entry = null,
		start,
		onsaved
	}: {
		open: boolean;
		/** The entry to change; without one, the dialog makes a new entry. */
		entry?: EnvironmentEntry | null;
		/** What a new entry starts as. */
		start?: Partial<Settings>;
		onsaved: (entry: EnvironmentEntry) => Promise<void>;
	} = $props();

	const blank: Settings = {
		kind: 'variable',
		target: '',
		secret: false,
		template: false,
		executable: false,
		applyToNew: true
	};
	let form = $state<Settings>({ ...blank });
	let value = $state<ValueInput | null>(null);
	let fresh = $state(0);

	$effect(() => {
		if (open) {
			untrack(() => {
				form = entry
					? {
							kind: entry.kind,
							target: entry.kind === 'file' ? `~/${entry.target}` : entry.target,
							secret: entry.secret,
							template: entry.template,
							executable: entry.executable,
							applyToNew: entry.applyToNew
						}
					: { ...blank, ...start };
				value = null;
				fresh += 1;
			});
		}
	});

	const variable = $derived(form.kind === 'variable');
	const problem = $derived(variable ? nameProblem(form.target) : pathProblem(form.target));
	const target = $derived(variable ? form.target : homePath(form.target));
	const reshaped = $derived(
		entry !== null && (entry.kind !== form.kind || entry.secret !== form.secret)
	);
	const ready = $derived(target !== '' && problem === null && (entry !== null || value !== null));

	async function save() {
		const settings = {
			kind: form.kind,
			target,
			secret: form.secret,
			template: !variable && form.template,
			executable: !variable && form.executable,
			applyToNew: form.applyToNew
		};
		const saved = entry
			? await environment.change(entry.id, settings)
			: await environment.create({ ...settings, value: value! });
		await onsaved(saved);
	}
</script>

{#snippet toggle(
	key: 'secret' | 'template' | 'executable' | 'applyToNew',
	label: string,
	hint: string
)}
	<Field.Field orientation="horizontal">
		<Switch id="entry-{key}" bind:checked={form[key]} aria-describedby="entry-{key}-hint" />
		<Field.Content>
			<Field.Label for="entry-{key}">{label}</Field.Label>
			<Field.Description id="entry-{key}-hint">{hint}</Field.Description>
		</Field.Content>
	</Field.Field>
{/snippet}

<FormDialog
	bind:open
	title={entry ? `Edit ${where(entry)}` : variable ? 'Add a variable' : 'Add a file'}
	description={entry
		? 'Its sandboxes and their overrides stay. Corral refuses a change that would keep one of them from starting, and says why.'
		: variable
			? 'Each SSH session of the sandboxes it is attached to gets it as an environment variable.'
			: 'Written into the home of the sandboxes it is attached to at each start, replacing what is at its path.'}
	submit={entry ? 'Save changes' : `Save ${noun(form.kind)}`}
	{ready}
	onsubmit={save}
>
	{#key fresh}
		<Field.Set>
			<Field.Legend variant="label">Kind</Field.Legend>
			<RadioGroup.Root bind:value={form.kind} class="grid grid-cols-2 gap-2">
				{#each [{ kind: 'variable', label: 'Variable' }, { kind: 'file', label: 'File' }] as choice (choice.kind)}
					<Field.Field orientation="horizontal">
						<RadioGroup.Item value={choice.kind} id="entry-kind-{choice.kind}" />
						<Field.Label for="entry-kind-{choice.kind}">{choice.label}</Field.Label>
					</Field.Field>
				{/each}
			</RadioGroup.Root>
		</Field.Set>

		{#if variable}
			<Field.Field data-invalid={problem ? true : undefined}>
				<Field.Label for="entry-target">Name</Field.Label>
				<Input
					id="entry-target"
					aria-describedby="entry-target-hint"
					aria-invalid={problem ? true : undefined}
					bind:value={form.target}
					required
					maxlength={64}
					autocomplete="off"
					autocapitalize="off"
					spellcheck={false}
					class="font-mono"
					placeholder="OPENAI_API_KEY"
				/>
				{#if problem}
					<Field.Error id="entry-target-hint">{problem}</Field.Error>
				{:else}
					<Field.Description id="entry-target-hint">
						Letters, digits and underscores, not starting with a digit. Case counts. PATH, HOME and
						the other names the sandbox sets, and names starting with CORRAL_, are reserved.
					</Field.Description>
				{/if}
			</Field.Field>
		{:else}
			<Field.Field data-invalid={problem ? true : undefined}>
				<Field.Label for="entry-target">Path in the home</Field.Label>
				<Input
					id="entry-target"
					aria-describedby="entry-target-hint"
					aria-invalid={problem ? true : undefined}
					bind:value={form.target}
					required
					maxlength={1040}
					autocomplete="off"
					autocapitalize="off"
					spellcheck={false}
					class="font-mono"
					placeholder="~/.config/tool/config.toml"
				/>
				{#if problem}
					<Field.Error id="entry-target-hint">{problem}</Field.Error>
				{:else}
					<Field.Description id="entry-target-hint">
						Inside /home/sandbox, as <code class="font-mono">~/.config/tool/config.toml</code>
						or <code class="font-mono">.config/tool/config.toml</code>. Missing directories are
						made.
					</Field.Description>
				{/if}
			</Field.Field>
		{/if}

		{#if !entry}
			<ValueField id="entry-value" kind={form.kind} secret={form.secret} name={target} bind:value />
		{/if}

		<Field.Group class="gap-4">
			{@render toggle(
				'secret',
				'Secret',
				entry
					? 'Never shown again. Changing it moves the value and its overrides to or from the secret store.'
					: `Kept in the secret store, never shown again${variable ? '' : ', readable by the sandbox user alone'}.`
			)}
			{#if !variable}
				{@render toggle(
					'template',
					'Template',
					'Each {{ NAME }}, an upper-case variable name, takes that variable’s value in the sandbox. Any other {{ stays as written.'
				)}
				{@render toggle(
					'executable',
					'Executable',
					form.secret ? 'Written with mode 0700.' : 'Written with mode 0755, for a script.'
				)}
			{/if}
			{@render toggle(
				'applyToNew',
				'Apply to new sandboxes',
				'Each sandbox you create from now on gets it.'
			)}
		</Field.Group>

		{#if reshaped}
			<p class="text-sm text-muted-foreground" role="status">
				Corral checks that its value, and each sandbox's override, suits a {form.secret
					? 'secret'
					: 'plain'}
				{noun(form.kind)} before it saves.
			</p>
		{/if}
	{/key}
</FormDialog>
