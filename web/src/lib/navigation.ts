import BoxesIcon from '@lucide/svelte/icons/boxes';
import CircleUserRoundIcon from '@lucide/svelte/icons/circle-user-round';
import SettingsIcon from '@lucide/svelte/icons/settings';
import SquareTerminalIcon from '@lucide/svelte/icons/square-terminal';
import UsersIcon from '@lucide/svelte/icons/users';
import VariableIcon from '@lucide/svelte/icons/variable';
import type { Me } from '#lib/api/generated/index.js';

export type NavItem = { href: string; label: string; icon: typeof BoxesIcon };

export type Navigation = { main: NavItem[]; admin: NavItem[] | null };

const main: NavItem[] = [
	{ href: '/sandboxes', label: 'Sandboxes', icon: SquareTerminalIcon },
	{ href: '/setup', label: 'Setup', icon: VariableIcon },
	{ href: '/account', label: 'Account', icon: CircleUserRoundIcon }
];

const admin: NavItem[] = [
	{ href: '/admin/settings', label: 'Settings', icon: SettingsIcon },
	{ href: '/admin/people', label: 'People', icon: UsersIcon },
	{ href: '/admin/sandboxes', label: 'All sandboxes', icon: BoxesIcon }
];

export function navigation(user: Me): Navigation {
	return { main, admin: user.role === 'admin' ? admin : null };
}

/** Whether the item is the current page or one of its children. */
export function isCurrent(item: NavItem, pathname: string): boolean {
	return pathname === item.href || pathname.startsWith(`${item.href}/`);
}

/** The item the current page belongs to, if any. */
export function currentItem(navigation: Navigation, pathname: string): NavItem | undefined {
	return [...navigation.main, ...(navigation.admin ?? [])].find((item) =>
		isCurrent(item, pathname)
	);
}
