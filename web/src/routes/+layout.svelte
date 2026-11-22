<script lang="ts">
	import './layout.css';
	import { ModeWatcher } from 'mode-watcher';
	import type { LayoutProps } from './$types';

	let { children }: LayoutProps = $props();
</script>

<!-- app.html applies the stored mode before the app starts; an injected script would never run. -->
<ModeWatcher disableHeadScriptInjection />
{@render children()}
