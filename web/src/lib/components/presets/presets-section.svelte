<script lang="ts">
	import BotIcon from '@lucide/svelte/icons/bot';
	import FolderGitIcon from '@lucide/svelte/icons/folder-git';
	import GitBranchIcon from '@lucide/svelte/icons/git-branch';
	import PlugIcon from '@lucide/svelte/icons/plug';
	import type { SetupEntry } from '#lib/api/generated/index.js';
	import DotfilesDialog from '#lib/components/presets/dotfiles-dialog.svelte';
	import GitDialog from '#lib/components/presets/git-dialog.svelte';
	import LlmDialog from '#lib/components/presets/llm-dialog.svelte';
	import McpDialog from '#lib/components/presets/mcp-dialog.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Item from '#lib/components/ui/item/index.js';
	import { dotfilesSummary, readDotfiles } from '#lib/presets/dotfiles.js';
	import { gitSummary, readGit } from '#lib/presets/git.js';
	import { llmSummary, readLlm } from '#lib/presets/llm.js';
	import { mcpSummary, readMcp } from '#lib/presets/mcp.js';

	let {
		entries,
		onchanged,
		onaddfile
	}: {
		entries: SetupEntry[];
		onchanged: () => Promise<void>;
		onaddfile: () => void;
	} = $props();

	type Preset = 'llm' | 'git' | 'mcp' | 'dotfiles';

	const open = $state<Record<Preset, boolean>>({
		llm: false,
		git: false,
		mcp: false,
		dotfiles: false
	});

	type Card = {
		key: Preset;
		title: string;
		Icon: typeof BotIcon;
		summary: string | null;
	};

	const presets = $derived<Card[]>([
		{
			key: 'llm',
			title: 'LLM providers',
			Icon: BotIcon,
			summary: llmSummary(readLlm(entries))
		},
		{
			key: 'git',
			title: 'Git identity',
			Icon: GitBranchIcon,
			summary: gitSummary(readGit(entries))
		},
		{
			key: 'mcp',
			title: 'MCP servers',
			Icon: PlugIcon,
			summary: mcpSummary(readMcp(entries))
		},
		{
			key: 'dotfiles',
			title: 'Dotfiles',
			Icon: FolderGitIcon,
			summary: dotfilesSummary(readDotfiles(entries))
		}
	]);
</script>

<section class="grid gap-3" aria-labelledby="presets-heading">
	<div class="grid gap-1">
		<h2 id="presets-heading" class="text-lg font-semibold tracking-tight">Presets</h2>
		<p class="text-sm text-muted-foreground">
			Forms for the common needs. What each saves is listed below, editable like any other entry.
		</p>
	</div>
	<ul class="grid gap-2 sm:grid-cols-2" aria-label="Presets">
		{#each presets as preset (preset.key)}
			<li>
				<Item.Root variant="outline" class="h-full">
					<Item.Media variant="icon">
						<preset.Icon class="text-muted-foreground" />
					</Item.Media>
					<Item.Content class="min-w-0">
						<Item.Title><h3>{preset.title}</h3></Item.Title>
						<Item.Description class="wrap-anywhere">
							{preset.summary ?? 'Not set up'}
						</Item.Description>
					</Item.Content>
					<Item.Actions>
						<Button variant="outline" size="sm" onclick={() => (open[preset.key] = true)}>
							{preset.summary ? 'Change' : 'Set up'}<span class="sr-only"> {preset.title}</span>
						</Button>
					</Item.Actions>
				</Item.Root>
			</li>
		{/each}
	</ul>
</section>

<LlmDialog bind:open={open.llm} {entries} {onchanged} />
<GitDialog bind:open={open.git} {entries} {onchanged} />
<McpDialog bind:open={open.mcp} {entries} {onchanged} />
<DotfilesDialog bind:open={open.dotfiles} {entries} {onchanged} {onaddfile} />
