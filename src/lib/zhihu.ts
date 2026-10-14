/**
 * 知乎 open-platform helpers for the built-in "知乎" Agent.
 *
 * The WebView never holds the plaintext Access Secret: it lives in durable
 * Host settings and is redacted to a `*`-mask over IPC. Mirrors
 * `mask_translate_api_key` in `src-tauri/.../settings/mod.rs`.
 */
import { commands } from "@/lib/core/bindings";
import { callApi } from "@/lib/core/ipc";
import { isTauri } from "@/lib/core/tauri";

/** 知乎开放平台个人中心，用于获取 / 重新生成 Access Secret。 */
export const ZHIHU_DEVELOPER_PROFILE_URL =
	"https://developer.zhihu.com/profile";

/** True when a non-empty secret is stored (plaintext or Host `*`-mask). */
export function hasZhihuSecret(secret: string | undefined): boolean {
	return Boolean(secret?.trim());
}

/**
 * Probe the configured Access Secret through the Host by running a minimal
 * Zhihu search. Consumes one call against the open-platform quota.
 */
export async function probeZhihuSecret(signal?: AbortSignal): Promise<boolean> {
	if (!isTauri()) return false;
	try {
		const ok = await callApi(() => commands.zhihuProbe(), {
			fallback: "Zhihu probe failed",
		});
		if (signal?.aborted) return false;
		return ok;
	} catch {
		return false;
	}
}
