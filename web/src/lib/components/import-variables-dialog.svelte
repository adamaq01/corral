<script lang="ts">
	import { untrack } from 'svelte';
	import { failureMessage } from '#lib/api/client.js';
	import { environment } from '#lib/api/environment.js';
	import type { EnvironmentEntry } from '#lib/api/generated/index.js';
	import FormDialog from '#lib/components/form-dialog.svelte';
	import * as Field from '#lib/components/ui/field/index.js';
	import { Switch } from '#lib/components/ui/switch/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';
	import { nameProblem, parseDotenv, variableTooLarge } from '#lib/environment.js';

	let {
		open = $bindable(),
		existing,
		onimported
	}: {
		open: boolean;
		/** The entries the user has, whose names an import skips. */
		existing: EnvironmentEntry[];
		onimported: (created: number) => Promise<void>;
	} = $props();

	let text = $state('');
	let secret = $state(true);
	let applyToNew = $state(true);
	let failures = $state<string[]>([]);

	$effect(() => {
		if (open) {
			untrack(() => {
				text = '';
				secret = true;
				applyToNew = true;
				failures = [];
			});
		}
	});

	const taken = $derived(
		new Set(existing.filter((entry) => entry.kind === 'variable').map((entry) => entry.target))
	);
	const parsed = $derived(parseDotenv(text));
	const rows = $derived(
		parsed.assignments.map((assignment, index) => ({
			...assignment,
			problem:
				nameProblem(assignment.name) ??
				(taken.has(assignment.name) ? 'you already have it' : null) ??
				(parsed.assignments.findIndex((other) => other.name === assignment.name) < index
					? 'set twice; the first one is imported'
					: null) ??
				(/[\r\n]/.test(assignment.value) ? 'more than one line' : null) ??
				variableTooLarge(assignment.name, assignment.value)
		}))
	);
	const ready = $derived(rows.filter((row) => row.problem === null));

	async function save() {
		failures = [];
		let created = 0;
		for (const row of ready) {
			try {
				await environment.create({
					kind: 'variable',
					target: row.name,
					secret,
					applyToNew,
					value: { text: row.value }
				});
				created += 1;
			} catch (failure) {
				failures.push(`${row.name}: ${failureMessage(failure)}`);
			}
		}
		await onimported(created);
		if (failures.length > 0) {
			throw new Error(`${created} imported; ${failures.length} refused. ${failures.join('. ')}`);
		}
	}
</script>

<FormDialog
	bind:open
	title="Import variables"
	description="Paste the lines of a .env file, NAME=value each. Names you already have are skipped."
	submit={ready.length === 0
		? 'Import'
		: `Import ${ready.length} ${ready.length === 1 ? 'variable' : 'variables'}`}
	ready={ready.length > 0}
	onsubmit={save}
>
	<Field.Field>
		<Field.Label for="import-text">Variables</Field.Label>
		<Textarea
			id="import-text"
			bind:value={text}
			rows={8}
			spellcheck={false}
			autocomplete="off"
			class="max-h-72 font-mono text-xs [-webkit-text-security:disc] focus:[-webkit-text-security:none]"
			placeholder="OPENAI_API_KEY=sk-...&#10;export GH_TOKEN=ghp_..."
			aria-describedby="import-hint"
		/>
		<Field.Description id="import-hint">
			Quotes around a value are removed; comments and blank lines are skipped. The text is shown
			only while the field has focus.
		</Field.Description>
	</Field.Field>
	{#if rows.length > 0 || parsed.problems.length > 0}
		<ul class="grid max-h-48 gap-1 overflow-y-auto text-sm" aria-label="What would be imported">
			{#each rows as row (row.line)}
				<li class="flex flex-wrap gap-x-2">
					<span class="font-mono break-all">{row.name}</span>
					<span class={row.problem ? 'text-destructive' : 'text-muted-foreground'}>
						{row.problem ? `skipped: ${row.problem}` : 'imported'}
					</span>
				</li>
			{/each}
			{#each parsed.problems as problem (problem.line)}
				<li class="text-destructive">Line {problem.line} is not NAME=value, so it is skipped.</li>
			{/each}
		</ul>
	{/if}
	<Field.Group class="gap-4">
		<Field.Field orientation="horizontal">
			<Switch id="import-secret" bind:checked={secret} />
			<Field.Label for="import-secret">Secret</Field.Label>
		</Field.Field>
		<Field.Field orientation="horizontal">
			<Switch id="import-new" bind:checked={applyToNew} />
			<Field.Label for="import-new">Apply to new sandboxes</Field.Label>
		</Field.Field>
	</Field.Group>
</FormDialog>
