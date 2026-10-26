import type {
	TranslateProviderConfig,
	TranslateSettings,
} from "@/lib/translate/types";

export const DEFAULT_TRANSLATE_SETTINGS: TranslateSettings = {
	/** Prefer Tencent Transmart: current no-key default with better availability. */
	provider: "tencenttransmart",
	targetLang: "ui",
	sourceLang: "auto",
	providerConfigs: {},
	autoTranslateSelection: false,
	dualPaneTranslate: true,
	// Keep the source PDF intact. Full-document translations open in the
	// companion panel next to the source instead of painting over the page.
	displayMode: "dualPane",
	dualPaneSource: "pdf",
	agentId: "",
	modelId: "",
	customPrompt: "",
	customSystem: "",
	customUser: "",
};

/** Blank commercial provider config (missing draft/stored entry fallback). */
export const EMPTY_TRANSLATE_PROVIDER_CONFIG: TranslateProviderConfig = {
	apiKey: "",
	baseUrl: "",
	region: "",
	model: "",
};
