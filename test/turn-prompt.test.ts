import type { TFunction } from "i18next";
import { describe, expect, it } from "vitest";
import { assembleTurnPrompt, paperContextBlock } from "@/lib/agent/turn-prompt";
import type { PaperMetadata } from "@/lib/paper";

const t = ((key: string) => key) as unknown as TFunction<"agent", undefined>;

const paper = {
	title: "Hilbert's sixth problem",
	authors: ["Yu Deng", "Zaher Hani"],
	year: 2025,
	publication: "arXiv",
	arxiv_id: "2503.01800",
	doi: null,
	abstract: "A derivation of fluid equations.",
} as unknown as PaperMetadata;

function assemble(extra: {
	isAcpCommand?: boolean;
	paper?: PaperMetadata | null;
}) {
	return assembleTurnPrompt({
		text: "总结当前论文",
		contextPaths: [],
		selections: [],
		visualDrafts: [],
		attachedImages: [],
		isAcpCommand: extra.isAcpCommand ?? false,
		paper: extra.paper ?? null,
		t,
	}).prompt;
}

describe("paperContextBlock", () => {
	it("includes public metadata and omits empty fields", () => {
		const block = paperContextBlock(paper, t);
		expect(block).toContain("composer.paperContext.heading");
		expect(block).toContain("Hilbert's sixth problem");
		expect(block).toContain("Yu Deng, Zaher Hani");
		expect(block).toContain("arXiv: 2503.01800");
		expect(block).toContain("A derivation of fluid equations.");
		// No DOI on this paper → no DOI line, and never a local path.
		expect(block).not.toContain("DOI:");
		expect(block).not.toContain("papers/");
	});

	it("caps the abstract and returns '' when nothing is known", () => {
		const huge = {
			...paper,
			abstract: "x".repeat(5000),
		} as PaperMetadata;
		const block = paperContextBlock(huge, t);
		expect(block.length).toBeLessThan(1600);
		expect(paperContextBlock({} as PaperMetadata, t)).toBe("");
	});
});

describe("assembleTurnPrompt paper context", () => {
	it("injects the paper block for a normal turn", () => {
		const prompt = assemble({ paper });
		expect(prompt).toContain("总结当前论文");
		expect(prompt).toContain("Hilbert's sixth problem");
	});

	it("omits the paper block for slash commands", () => {
		const prompt = assemble({ paper, isAcpCommand: true });
		expect(prompt).toBe("总结当前论文");
	});

	it("omits the paper block when no paper is focused", () => {
		expect(assemble({ paper: null })).toBe("总结当前论文");
	});
});
