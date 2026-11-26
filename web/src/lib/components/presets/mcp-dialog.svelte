<script lang="ts">
	import PlusIcon from '@lucide/svelte/icons/plus';
	import { tick, untrack } from 'svelte';
	import { toast } from 'svelte-sonner';
	import type { SetupEntry } from '#lib/api/generated/index.js';
	import FormDialog from '#lib/components/form-dialog.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Field from '#lib/components/ui/field/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import * as RadioGroup from '#lib/components/ui/radio-group/index.js';
	import { Switch } from '#lib/components/ui/switch/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';
	import { applyChanges } from '#lib/presets/apply.js';
	import { readMcp, writeMcp, type McpServer, type McpState } from '#lib/presets/mcp.js';

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

	type Runs = McpServer['runs'];

	/** A server as the member fills it in, its lists one item per line. */
	type Row = {
		key: number;
		name: string;
		runs: Runs;
		command: string;
		args: string;
		env: string;
		url: string;
		headers: string;
	};

	/** A secret list read back is kept until the member replaces it. */
	type Form = { secret: boolean; replacing: boolean; rows: Row[] };

	const NAME_RULE = 'Letters, digits, - and _.';
	const CHOICES: { runs: Runs; label: string }[] = [
		{ runs: 'command', label: 'A command' },
		{ runs: 'url', label: 'A URL' }
	];

	let made = 0;

	function row(server?: McpServer): Row {
		const blank = { key: made++, name: '', command: '', args: '', env: '', url: '', headers: '' };
		if (server === undefined) {
			return { ...blank, runs: 'command' };
		}
		if (server.runs === 'command') {
			return {
				...blank,
				name: server.name,
				runs: 'command',
				command: server.command,
				args: server.args.join('\n'),
				env: joined(server.env, '=')
			};
		}
		return {
			...blank,
			name: server.name,
			runs: 'url',
			url: server.url,
			headers: joined(server.headers, ': ')
		};
	}

	function fresh(state: McpState): Form {
		if (state.secret) {
			return { secret: true, replacing: false, rows: [] };
		}
		return {
			secret: false,
			replacing: false,
			rows: state.servers.length > 0 ? state.servers.map((server) => row(server)) : [row()]
		};
	}

	function joined(pairs: Record<string, string>, separator: string): string {
		return Object.entries(pairs)
			.map(([name, value]) => `${name}${separator}${value}`)
			.join('\n');
	}

	function lines(text: string): string[] {
		return text
			.split('\n')
			.map((line) => line.trim())
			.filter(Boolean);
	}

	/** The `name<separator>value` lines of `text`, or `null` when one is not. */
	function pairs(text: string, separator: string): Record<string, string> | null {
		const read: [string, string][] = [];
		for (const line of lines(text)) {
			const at = line.indexOf(separator);
			const name = line.slice(0, Math.max(at, 0)).trim();
			if (name === '') {
				return null;
			}
			read.push([name, line.slice(at + 1).trim()]);
		}
		return Object.fromEntries(read);
	}

	function isAddress(text: string): boolean {
		return URL.canParse(text) && ['http:', 'https:'].includes(new URL(text).protocol);
	}

	const read = $derived(readMcp(entries));
	let form = $state(fresh({ secret: false, servers: [] }));

	$effect(() => {
		if (open) {
			untrack(() => {
				form = fresh(read);
			});
		}
	});

	const kept = $derived(read.secret && !form.replacing);
	const checked = $derived(
		form.rows.map((row, index) => {
			const name = row.name.trim();
			const env = pairs(row.env, '=');
			const headers = pairs(row.headers, ':');
			const url = row.url.trim();
			const problems = {
				name:
					name === ''
						? null
						: !/^[A-Za-z0-9_-]+$/.test(name)
							? NAME_RULE
							: form.rows.findIndex((other) => other.name.trim() === name) < index
								? 'Another server has this name.'
								: null,
				env: env === null ? 'Each line as NAME=value.' : null,
				url: url !== '' && !isAddress(url) ? 'An address starting with https:// or http://.' : null,
				headers: headers === null ? 'Each line as Name: value.' : null
			};
			const server: McpServer =
				row.runs === 'command'
					? {
							name,
							runs: 'command',
							command: row.command.trim(),
							args: lines(row.args),
							env: env ?? {}
						}
					: { name, runs: 'url', url, headers: headers ?? {} };
			const complete =
				name !== '' &&
				problems.name === null &&
				(server.runs === 'command'
					? server.command !== '' && problems.env === null
					: url !== '' && problems.url === null && problems.headers === null);
			return { server, problems, complete };
		})
	);
	const changes = $derived(
		kept
			? []
			: writeMcp(entries, { secret: form.secret, servers: checked.map(({ server }) => server) })
	);
	const ready = $derived(changes.length > 0 && checked.every(({ complete }) => complete));

	async function focusName(index: number) {
		await tick();
		document.getElementById(`mcp-${form.rows[index].key}-name`)?.focus();
	}

	async function replace() {
		form.replacing = true;
		form.rows = [row()];
		await focusName(0);
	}

	async function add() {
		form.rows.push(row());
		await focusName(form.rows.length - 1);
	}

	async function save() {
		try {
			await applyChanges(changes);
		} finally {
			await onchanged();
		}
		toast.success('Saved the MCP servers');
	}
</script>

{#snippet perLine(
	row: Row,
	field: 'args' | 'env' | 'headers',
	label: string,
	hint: string,
	problem: string | null
)}
	{@const id = `mcp-${row.key}-${field}`}
	<Field.Field data-invalid={problem ? true : undefined}>
		<Field.Label for={id}>{label}</Field.Label>
		<Textarea
			{id}
			bind:value={row[field]}
			rows={2}
			autocomplete="off"
			autocapitalize="none"
			spellcheck={false}
			class="font-mono text-xs"
			aria-invalid={problem ? true : undefined}
			aria-describedby="{id}-hint"
		/>
		{#if problem}
			<Field.Error id="{id}-hint">{problem}</Field.Error>
		{:else}
			<Field.Description id="{id}-hint">{hint}</Field.Description>
		{/if}
	</Field.Field>
{/snippet}

<FormDialog
	bind:open
	title="MCP servers"
	description="Applied to Claude Code, Codex, Pi and oh-my-pi at each start, as their user-scope servers."
	submit="Save servers"
	{ready}
	onsubmit={save}
>
	{#if kept}
		<Field.Field>
			<Field.Title>Servers</Field.Title>
			<div class="flex h-8 items-center justify-between gap-2 rounded-lg border px-2.5">
				<span class="text-sm text-muted-foreground">Set, hidden</span>
				<Button variant="link" size="sm" class="h-auto p-0" onclick={replace}>
					Replace<span class="sr-only"> the servers</span>
				</Button>
			</div>
		</Field.Field>
	{:else}
		{#each form.rows as row, index (row.key)}
			{@const id = `mcp-${row.key}`}
			{@const { problems } = checked[index]}
			<Card.Root size="sm">
				<Card.Content>
					<Field.Set>
						<Field.Legend>Server {index + 1}</Field.Legend>
						<Field.Field data-invalid={problems.name ? true : undefined}>
							<Field.Label for="{id}-name">Name</Field.Label>
							<Input
								id="{id}-name"
								bind:value={row.name}
								placeholder="fetch"
								autocomplete="off"
								autocapitalize="none"
								spellcheck={false}
								class="font-mono"
								aria-invalid={problems.name ? true : undefined}
								aria-describedby="{id}-name-hint"
							/>
							{#if problems.name}
								<Field.Error id="{id}-name-hint">{problems.name}</Field.Error>
							{:else}
								<Field.Description id="{id}-name-hint">{NAME_RULE}</Field.Description>
							{/if}
						</Field.Field>
						<Field.Set>
							<Field.Legend variant="label">Runs as</Field.Legend>
							<RadioGroup.Root bind:value={row.runs} class="grid grid-cols-2 gap-2">
								{#each CHOICES as choice (choice.runs)}
									<Field.Field orientation="horizontal">
										<RadioGroup.Item value={choice.runs} id="{id}-runs-{choice.runs}" />
										<Field.Label for="{id}-runs-{choice.runs}">{choice.label}</Field.Label>
									</Field.Field>
								{/each}
							</RadioGroup.Root>
						</Field.Set>
						{#if row.runs === 'command'}
							<Field.Field>
								<Field.Label for="{id}-command">Command</Field.Label>
								<Input
									id="{id}-command"
									bind:value={row.command}
									placeholder="uvx"
									autocomplete="off"
									autocapitalize="none"
									spellcheck={false}
									class="font-mono"
								/>
							</Field.Field>
							{@render perLine(row, 'args', 'Arguments', 'One per line.', null)}
							{@render perLine(
								row,
								'env',
								'Variables',
								'One per line, as NAME=value.',
								problems.env
							)}
						{:else}
							<Field.Field data-invalid={problems.url ? true : undefined}>
								<Field.Label for="{id}-url">URL</Field.Label>
								<Input
									id="{id}-url"
									type="url"
									bind:value={row.url}
									placeholder="https://mcp.example.com/mcp"
									autocomplete="off"
									autocapitalize="none"
									spellcheck={false}
									aria-invalid={problems.url ? true : undefined}
									aria-describedby={problems.url ? `${id}-url-hint` : undefined}
								/>
								{#if problems.url}
									<Field.Error id="{id}-url-hint">{problems.url}</Field.Error>
								{/if}
							</Field.Field>
							{@render perLine(
								row,
								'headers',
								'Headers',
								'One per line, as Name: value.',
								problems.headers
							)}
						{/if}
						<Button
							variant="link"
							size="sm"
							class="h-auto w-fit p-0"
							onclick={() => form.rows.splice(index, 1)}
						>
							Remove<span class="sr-only"> server {index + 1}</span>
						</Button>
					</Field.Set>
				</Card.Content>
			</Card.Root>
		{/each}
		<Button variant="outline" size="sm" class="w-fit" onclick={add}>
			<PlusIcon />
			Add server
		</Button>
	{/if}
	<Field.Field orientation="horizontal">
		<Switch
			id="mcp-secret"
			bind:checked={form.secret}
			disabled={kept}
			aria-describedby="mcp-secret-hint"
		/>
		<Field.Content>
			<Field.Label for="mcp-secret">Secret</Field.Label>
			<Field.Description id="mcp-secret-hint">
				Keeps tokens in headers or variables hidden; the list cannot be read back here once it is
				secret, only replaced.
			</Field.Description>
		</Field.Content>
	</Field.Field>
</FormDialog>
