<script lang="ts">
	import CoffeeIcon from '@lucide/svelte/icons/coffee';
	import { toast } from 'svelte-sonner';
	import { failureMessage } from '#lib/api/client.js';
	import type { Sandbox } from '#lib/api/generated/index.js';
	import { sandboxes } from '#lib/api/sandboxes.js';
	import Timestamp from '#lib/components/timestamp.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import * as NativeSelect from '#lib/components/ui/native-select/index.js';
	import { duration, keepAwakeChoices, timeoutChoices } from '#lib/idle.js';

	let { sandbox, onchanged }: { sandbox: Sandbox; onchanged: () => void } = $props();

	const idle = $derived(sandbox.idle);
	const DEFAULT = 'default';
	let saving = $state(false);
	let failure = $state<string | null>(null);

	const picked = $derived(idle.chosen ? String(idle.timeoutSecs) : DEFAULT);
	const keptAwake = $derived(
		idle.keepAwakeUntil !== null && new Date(idle.keepAwakeUntil).getTime() > Date.now()
	);

	async function save(change: () => Promise<unknown>, done: string) {
		saving = true;
		failure = null;
		try {
			await change();
			toast.success(done);
		} catch (error) {
			failure = failureMessage(error);
		} finally {
			saving = false;
		}
		onchanged();
	}

	function chooseTimeout(value: string) {
		const secs = value === DEFAULT ? null : Number(value);
		const said =
			secs === null
				? `${sandbox.name} sleeps after the default ${duration(idle.defaultTimeoutSecs)} with no work`
				: `${sandbox.name} sleeps after ${duration(secs)} with no work`;
		return save(() => sandboxes.setIdleTimeout(sandbox.name, secs), said);
	}

	function keepAwake(until: Date | null) {
		const said =
			until === null
				? `${sandbox.name} sleeps again once idle`
				: `${sandbox.name} stays awake until ${until.toLocaleString()}`;
		return save(() => sandboxes.keepAwake(sandbox.name, until?.toISOString() ?? null), said);
	}
</script>

<Card.Root>
	<Card.Header>
		<Card.Title>Sleep</Card.Title>
		<Card.Description>
			{sandbox.name} sleeps after {duration(idle.timeoutSecs)} with no work: no typing, commands, file
			transfers or forwarded traffic through SSH, and next to no processor or network use inside it. An
			open but untouched connection does not keep it awake. Asleep, its home is kept and its programs
			end.
		</Card.Description>
	</Card.Header>
	<Card.Content class="grid gap-4">
		<div class="grid max-w-xs gap-2">
			<Label for="idle-timeout">Sleep after no work for</Label>
			<NativeSelect.Root
				id="idle-timeout"
				value={picked}
				disabled={saving}
				onchange={(event) => chooseTimeout(event.currentTarget.value)}
			>
				<NativeSelect.Option value={DEFAULT}>
					{duration(idle.defaultTimeoutSecs)} (default)
				</NativeSelect.Option>
				{#each timeoutChoices(idle) as secs (secs)}
					<NativeSelect.Option value={String(secs)}>{duration(secs)}</NativeSelect.Option>
				{/each}
			</NativeSelect.Root>
		</div>
		<div class="grid gap-2">
			<p class="text-sm font-medium">Keep awake</p>
			{#if keptAwake && idle.keepAwakeUntil}
				<p class="text-sm text-muted-foreground" role="status">
					Awake until <Timestamp value={idle.keepAwakeUntil} />, with or without work.
				</p>
			{:else}
				<p class="text-sm text-muted-foreground">
					For an idle agent you want left alone. <code class="font-mono text-xs">
						corral keep-awake 4h
					</code> inside does the same.
				</p>
			{/if}
			<div class="flex flex-wrap gap-2">
				{#each keepAwakeChoices(idle, new Date()) as choice (choice.label)}
					<Button
						variant="outline"
						size="sm"
						disabled={saving}
						onclick={() => keepAwake(choice.until)}
					>
						<CoffeeIcon />
						{choice.label}
					</Button>
				{/each}
				{#if keptAwake}
					<Button variant="ghost" size="sm" disabled={saving} onclick={() => keepAwake(null)}>
						Stop keeping awake
					</Button>
				{/if}
			</div>
		</div>
		{#if failure}
			<p class="text-sm text-destructive first-letter:uppercase" role="alert">{failure}</p>
		{/if}
	</Card.Content>
</Card.Root>
