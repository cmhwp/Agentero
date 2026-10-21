import { commands } from "@/lib/core/bindings";
import { callApiResult } from "@/lib/core/ipc";
import { isTauri } from "@/lib/core/tauri";

export type MemberLayout = {
	shared: boolean;
	userId: string | null;
};

const PERSONAL: MemberLayout = { shared: false, userId: null };
const TTL_MS = 2_000;
const cache = new Map<string, { at: number; layout: MemberLayout }>();

/** Drop the short-lived layout cache after sign-in, sign-out, or a roster refresh. */
export function invalidateMemberLayout(): void {
	cache.clear();
}

/**
 * Shared-library layout for a vault path or any file inside it.
 * Personal and org-grant libraries stay on the flat mark layout.
 */
export async function memberLayout(path: string): Promise<MemberLayout> {
	if (!path || !isTauri()) return PERSONAL;
	const hit = cache.get(path);
	if (hit && Date.now() - hit.at < TTL_MS) return hit.layout;
	const data = await callApiResult(() => commands.libraryMemberLayout({ path }));
	const layout: MemberLayout = {
		shared: data.shared,
		userId: data.userId,
	};
	cache.set(path, { at: Date.now(), layout });
	return layout;
}
