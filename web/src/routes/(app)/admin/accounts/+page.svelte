<script lang="ts">
	import UserCogIcon from '@lucide/svelte/icons/user-cog';
	import PageHeader from '#lib/components/page-header.svelte';
	import * as Empty from '#lib/components/ui/empty/index.js';
	import AccountsManager from './accounts-manager.svelte';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	const url = $derived(data.providers.find((provider) => provider.accounts !== null)?.accounts);
</script>

{#if url}
	<AccountsManager {url} />
{:else}
	<div class="space-y-6">
		<PageHeader title="Local accounts" description="The accounts that sign in with a password." />
		<Empty.Root class="border">
			<Empty.Header>
				<Empty.Media variant="icon"><UserCogIcon /></Empty.Media>
				<Empty.Title>Local accounts are off</Empty.Title>
				<Empty.Description>
					Corral keeps accounts of its own when its config has an <code>[identity.local]</code>
					section.
				</Empty.Description>
			</Empty.Header>
		</Empty.Root>
	</div>
{/if}
