<script lang="ts">
	import { goto } from '$app/navigation';
	import { ApiError } from '#lib/api/client.js';
	import { passwordLogin } from '#lib/api/auth.js';
	import type { Provider } from '#lib/api/generated/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';

	let { provider, next }: { provider: Provider; next: string } = $props();

	let username = $state('');
	let password = $state('');
	let pending = $state(false);
	let error = $state<string | null>(null);

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		pending = true;
		error = null;
		try {
			await passwordLogin(provider, { username, password });
			await goto(next, { replace: true });
		} catch (failure) {
			error = failure instanceof ApiError ? failure.message : 'Signing in failed';
			password = '';
		} finally {
			pending = false;
		}
	}
</script>

<form class="grid gap-4" onsubmit={submit} aria-label={provider.label}>
	<div class="grid gap-2">
		<Label for="{provider.id}-username">Username</Label>
		<Input
			id="{provider.id}-username"
			autocomplete="username"
			autocapitalize="none"
			spellcheck={false}
			required
			bind:value={username}
		/>
	</div>
	<div class="grid gap-2">
		<Label for="{provider.id}-password">Password</Label>
		<Input
			id="{provider.id}-password"
			type="password"
			autocomplete="current-password"
			required
			bind:value={password}
		/>
	</div>
	{#if error}
		<p class="text-sm text-destructive first-letter:uppercase" role="alert">{error}</p>
	{/if}
	<Button type="submit" disabled={pending}>
		{pending ? 'Signing in…' : 'Sign in'}
	</Button>
</form>
