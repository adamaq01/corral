<script lang="ts">
	import { beforeNavigate, goto } from '$app/navigation';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';

	/** While `dirty` holds, leaving the page asks first. */
	let { dirty, what }: { dirty: boolean; what: string } = $props();

	let leavingTo = $state<URL | null>(null);
	let leaving = false;

	beforeNavigate((navigation) => {
		if (!dirty || leaving) {
			return;
		}
		navigation.cancel();
		// Leaving the app, cancelling has the browser ask instead.
		if (navigation.type !== 'leave' && navigation.to) {
			leavingTo = navigation.to.url;
		}
	});

	async function leave() {
		const to = leavingTo;
		leavingTo = null;
		if (to) {
			leaving = true;
			try {
				await goto(to);
			} finally {
				leaving = false;
			}
		}
	}
</script>

<AlertDialog.Root bind:open={() => leavingTo !== null, (open) => !open && (leavingTo = null)}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>Leave without saving?</AlertDialog.Title>
			<AlertDialog.Description>Your changes to {what} are not saved.</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>Stay</AlertDialog.Cancel>
			<AlertDialog.Action onclick={leave}>Leave</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
