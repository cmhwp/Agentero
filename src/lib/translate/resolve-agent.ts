/**
 * Resolve which Agent seat + model to use for translation.
 * Independent of Chat's currently selected agent.
 */
import {
	type AgentDescriptor,
	type AgentListResponse,
	isChatOnlyAgentTemplate,
	listAgents,
	loadModelPref,
} from "@/lib/agent";
import { loadSettings } from "@/lib/settings";
import type { TranslateSettings } from "@/lib/translate/types";

export type ResolvedTranslateAgent = {
	agentId: string | undefined;
	modelId: string | undefined;
};

/**
 * @param settings translate settings slice
 * @param registry optional listAgents() result for defaultId fallback
 */
export function resolveTranslateAgent(
	settings: Pick<TranslateSettings, "agentId" | "modelId">,
	registry?: Pick<AgentListResponse, "defaultId" | "agents"> | null,
): ResolvedTranslateAgent {
	const preferred = settings.agentId?.trim() || "";
	let agentId: string | undefined = preferred || undefined;

	if (!agentId) {
		agentId = registry?.defaultId ?? undefined;
	}

	// If preferred id is gone from registry, fall back to default
	if (agentId && registry?.agents?.length) {
		const found = registry.agents.find((a) => a.id === agentId);
		if (!found?.available) {
			const def = registry.defaultId;
			const defOk = def
				? registry.agents.find((a) => a.id === def && a.available)
				: undefined;
			agentId = defOk?.id;
		}
	}

	if (!agentId) {
		return { agentId: undefined, modelId: undefined };
	}

	const pinnedModel = settings.modelId?.trim() || "";
	const modelId = pinnedModel || loadModelPref(agentId) || undefined;

	return { agentId, modelId: modelId || undefined };
}

/**
 * Agents that can actually run (for Settings Select).
 *
 * Built-in chat-only agents (e.g. the Zhihu agent) are excluded: they cannot
 * read Vault files or run ACP prompts, so they are not valid seats for
 * translation / PDF-ask.
 */
export function listAvailableAgents(
	registry: AgentListResponse | null | undefined,
): AgentDescriptor[] {
	if (!registry?.agents?.length) return [];
	return registry.agents.filter(
		(a) => a.available && !isChatOnlyAgentTemplate(a.template),
	);
}

/**
 * Resolve the translate-settings agent seat against the live registry (a
 * failed registry fetch degrades to settings-only resolution).
 */
export async function resolveConfiguredTranslateAgent(): Promise<ResolvedTranslateAgent> {
	const registry = await listAgents().catch(() => null);
	return resolveTranslateAgent(loadSettings().translate, registry);
}
