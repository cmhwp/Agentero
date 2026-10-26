import { pluginsSurfaceEnabled } from "@/lib/plugins/visibility";

/**
 * Personal-fork switches for desktop surfaces that are compiled in but not
 * used. Host commands stay registered; only user-facing entry points are
 * hidden, so flipping a flag back to `true` restores the original UI.
 */

/** "Open remote…" (SSH/SFTP Vault) in the welcome screen and Vault switcher. */
export const remoteVaultSurfaceEnabled: boolean = false;

/** Settings → Remote access (mobile pairing / Bridge relay). */
export const remoteAccessSurfaceEnabled: boolean = false;

/** S3 / WebDAV backends in Settings → Sync. Git sync is always available. */
export const cloudSyncSurfaceEnabled: boolean = false;

const HIDDEN_SETTINGS_SECTIONS = new Set<string>([
	...(pluginsSurfaceEnabled ? [] : ["plugins"]),
	...(remoteAccessSurfaceEnabled ? [] : ["remote-access"]),
]);

export function isSettingsSectionVisible(section: string): boolean {
	return !HIDDEN_SETTINGS_SECTIONS.has(section);
}
