<script lang="ts">
	import './layout.css';
	import { ModeWatcher } from 'mode-watcher';
	import { Toaster } from '#lib/components/ui/sonner/index.js';
	import type { LayoutProps } from './$types';

	let { children }: LayoutProps = $props();
</script>

<!-- app.html applies the stored mode before the app starts; an injected script would never run. -->
<ModeWatcher disableHeadScriptInjection />
<Toaster position="bottom-right" />
{@render children()}
