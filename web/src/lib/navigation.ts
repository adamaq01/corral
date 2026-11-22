import BoxesIcon from '@lucide/svelte/icons/boxes';
import KeyRoundIcon from '@lucide/svelte/icons/key-round';
import LockKeyholeIcon from '@lucide/svelte/icons/lock-keyhole';
import SquareTerminalIcon from '@lucide/svelte/icons/square-terminal';
import UserCogIcon from '@lucide/svelte/icons/user-cog';
import UsersIcon from '@lucide/svelte/icons/users';
import type { Me, Provider } from '#lib/api/generated/index.js';

export type NavItem = { href: string; label: string; icon: typeof BoxesIcon };

export type Navigation = { main: NavItem[]; admin: NavItem[] | null };

export function navigation(user: Me, providers: Provider[]): Navigation {
	const main = [
		{ href: '/sandboxes', label: 'Sandboxes', icon: SquareTerminalIcon },
		{ href: '/keys', label: 'SSH keys', icon: KeyRoundIcon },
		{ href: '/credentials', label: 'Credentials', icon: LockKeyholeIcon }
	];
	if (user.role !== 'admin') {
		return { main, admin: null };
	}
	const accounts = providers.some((provider) => provider.accounts !== null);
	const admin = [
		{ href: '/admin/users', label: 'Users', icon: UsersIcon },
		...(accounts ? [{ href: '/admin/accounts', label: 'Local accounts', icon: UserCogIcon }] : []),
		{ href: '/admin/sandboxes', label: 'All sandboxes', icon: BoxesIcon }
	];
	return { main, admin };
}

/** Whether the item is the current page or one of its children. */
export function isCurrent(item: NavItem, pathname: string): boolean {
	return pathname === item.href || pathname.startsWith(`${item.href}/`);
}
