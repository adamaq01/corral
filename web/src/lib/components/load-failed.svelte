<script lang="ts">
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import { Button } from '#lib/components/ui/button/index.js';

	let { title, error, onretry }: { title: string; error: string; onretry: () => void } = $props();
</script>

<Alert.Root variant="destructive">
	<CircleAlertIcon />
	<Alert.Title>{title}</Alert.Title>
	<Alert.Description class="first-letter:uppercase">{error}</Alert.Description>
	<Alert.Action>
		<Button variant="outline" size="sm" onclick={onretry}>Retry</Button>
	</Alert.Action>
</Alert.Root>
