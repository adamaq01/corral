<script lang="ts">
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import { redirectLogin } from '#lib/api/auth.js';
	import BrandMark from '#lib/components/brand-mark.svelte';
	import PasswordSignIn from '#lib/components/password-sign-in.svelte';
	import ThemeToggle from '#lib/components/theme-toggle.svelte';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import { buttonVariants } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Separator } from '#lib/components/ui/separator/index.js';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
</script>

<svelte:head>
	<title>Sign in · Corral</title>
</svelte:head>

<div class="flex min-h-svh flex-col items-center justify-center gap-6 bg-muted p-4 md:p-10">
	<div class="absolute top-4 right-4">
		<ThemeToggle />
	</div>
	<div class="flex items-center gap-2 text-lg font-semibold">
		<BrandMark />
		Corral
	</div>
	<Card.Root class="w-full max-w-sm">
		<Card.Header>
			<Card.Title class="text-xl">Sign in</Card.Title>
			<Card.Description>Reach your sandboxes, keys and credentials.</Card.Description>
		</Card.Header>
		<Card.Content class="grid gap-6">
			{#if data.error}
				<Alert.Root variant="destructive">
					<CircleAlertIcon />
					<Alert.Title>Signing in failed</Alert.Title>
					<Alert.Description class="first-letter:uppercase">{data.error}</Alert.Description>
				</Alert.Root>
			{/if}
			{#each data.providers as provider, index (provider.id)}
				{#if index > 0}
					<div class="flex items-center gap-3 text-xs text-muted-foreground uppercase">
						<Separator class="flex-1" />
						or
						<Separator class="flex-1" />
					</div>
				{/if}
				{#if provider.kind === 'redirect'}
					<a
						class={buttonVariants({ variant: 'outline' })}
						href={redirectLogin(provider, data.next)}
						data-sveltekit-reload
					>
						Sign in with {provider.label}
					</a>
				{:else}
					<PasswordSignIn {provider} next={data.next} />
				{/if}
			{/each}
		</Card.Content>
	</Card.Root>
</div>
