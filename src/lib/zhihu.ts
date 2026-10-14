/**
 * 知乎 open-platform helpers for the built-in "知乎" Agent.
 *
 * The WebView never holds the plaintext Access Secret: it lives in durable
 * Host settings and is redacted to a `*`-mask over IPC. Mirrors
 * `mask_translate_api_key` in `src-tauri/.../settings/mod.rs`.
 */
import {
	commands,
	type QuestionDraftInput,
	type ZhihuQuestionDraft,
} from "@/lib/core/bindings";
import { callApi } from "@/lib/core/ipc";
import { isTauri } from "@/lib/core/tauri";

/** 知乎开放平台个人中心，用于获取 / 重新生成 Access Secret。 */
export const ZHIHU_DEVELOPER_PROFILE_URL =
	"https://developer.zhihu.com/profile";

/** 知乎站点入口：提问草稿复制到剪贴板后打开，用户粘贴发布。 */
export const ZHIHU_HOME_URL = "https://www.zhihu.com/";

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

/**
 * Build a Zhihu question draft for a paper. The Host translates the title to
 * Chinese via Zhida when a secret is configured (best effort; falls back to
 * the original title) and returns `{ title, body }`. The caller copies it and
 * opens Zhihu — a desktop app cannot drive Zhihu's ask modal.
 */
export async function buildZhihuQuestionDraft(
	input: QuestionDraftInput,
): Promise<ZhihuQuestionDraft> {
	return callApi(() => commands.zhihuQuestionDraft(input), {
		fallback: "Zhihu question draft failed",
		desktopOnly: "Zhihu question draft requires the desktop app",
	});
}
