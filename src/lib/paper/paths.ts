import { normalizePath } from "@/lib/core/path";

/** True when path is the `papers` directory itself (Vault-relative or absolute). */
export function isPapersRoot(path: string | null): boolean {
	if (!path) return false;
	const norm = normalizePath(path);
	return /(^|\/)papers$/i.test(norm);
}

/**
 * True when path is somewhere under a `papers` root (not the root itself).
 * Absolute: `…/papers/…` ; Vault-relative: `papers/…`.
 */
export function isUnderPapers(path: string | null): boolean {
	if (!path || isPapersRoot(path)) return false;
	const norm = normalizePath(path);
	return /(^|\/)papers\//i.test(norm);
}

/** User-facing extras under a paper unit (`{paper}/attachments/…`). */
export const PAPER_ATTACHMENTS_DIR = "attachments";

/** Cloned code repositories under a paper unit (`{paper}/code/<repo>/…`). */
export const PAPER_CODE_DIR = "code";

/**
 * Directories that live inside a paper unit. They are never nested-paper
 * candidates, and writes under them do not change catalog rows.
 */
export const PAPER_INTERNAL_DIR_NAMES = [
	"source",
	"assets",
	"marks",
	PAPER_ATTACHMENTS_DIR,
	PAPER_CODE_DIR,
] as const;

export function isPaperInternalDirName(
	name: string | null | undefined,
): boolean {
	if (!name) return false;
	return (PAPER_INTERNAL_DIR_NAMES as readonly string[]).includes(
		name.toLowerCase(),
	);
}

export function isPaperAttachmentsDirName(
	name: string | null | undefined,
): boolean {
	return name?.toLowerCase() === PAPER_ATTACHMENTS_DIR;
}

/**
 * True when path is inside a paper folder's internal dirs
 * (`<paper>/source|assets|marks|attachments|code/…`). Highlight/LaTeX/image/
 * attachment/code writes there never change catalog rows, so they must not
 * trigger library refreshes.
 */
export function isPaperAssetPath(path: string | null): boolean {
	if (!path) return false;
	const norm = normalizePath(path);
	return /(^|\/)papers\/.+?\/(source|assets|marks|attachments|code)(\/|$)/i.test(
		norm,
	);
}

/** True when path is inside a cloned repository (`<paper>/code/<repo>/…`). */
export function isPaperCodePath(path: string | null): boolean {
	if (!path) return false;
	const norm = normalizePath(path);
	return /(^|\/)papers\/(?:[^/]+\/)+?code\/[^/]+(?:\/|$)/i.test(norm);
}

/** `<paperDir>/NOTES.md` — structured notes for the paper. */
export function notesPathForPaper(paperDir: string): string {
	const sep = paperDir.endsWith("/") ? "" : "/";
	return `${paperDir}${sep}NOTES.md`;
}
