import type {
	PdfAnnotationObject,
	PdfHighlightAnnoObject,
} from "@embedpdf/models";
import { PdfAnnotationSubtype } from "@embedpdf/models";
import type { TrackedAnnotation } from "@embedpdf/plugin-annotation";
import type { AnnotationTransferItem } from "@embedpdf/plugin-annotation/react";
import { isTauri } from "@/lib/core/tauri";
import { memberLayout } from "@/lib/library/member-layout";
import {
	type HighlightColor,
	highlightColorFromHex,
} from "@/lib/pdf/highlight/palette";
import { sanitizeTranslatedRects } from "@/lib/pdf/highlight/translated-geometry";
import type { PdfHighlight } from "@/lib/pdf/highlight/types";
import {
	ANNOTATIONS_JSON,
	MARKS_FOLDER,
	listMarkJsonFiles,
	markSelfWrite,
} from "@/lib/pdf/selection/marks-io";
import {
	joinVaultPath,
	readVaultFile,
	removeVaultPath,
	writeVaultFile,
} from "@/lib/vault";

/** Items loaded from `annotations.json` or another member. Not rewritten on save. */
const AGGREGATE_OWNER = "\0";
const ownersByPaper = new Map<string, Map<string, string>>();

/**
 * Highlights/批注 are EmbedPDF annotation transfer items.
 * Personal libraries keep them in `papers/<id>/marks/annotations.json`.
 * Shared libraries keep one file per member at `marks/{userId}/{id}.json`
 * and build the viewer list from every member's files.
 * Ask/Translate stay as kind files beside those paths.
 * A legacy root-level `papers/<id>/annotations.json` is migrated into
 * `marks/` on first load for personal libraries.
 */
export const ANNOTATIONS_FILE = ANNOTATIONS_JSON;

/** In-memory fallback when not running under Tauri (browser dev). */
const memoryStore = new Map<string, AnnotationTransferItem[]>();

function dedupeAnnotationItems(
	items: AnnotationTransferItem[],
): AnnotationTransferItem[] {
	const seen = new Set<string>();
	const out: AnnotationTransferItem[] = [];
	for (const item of items) {
		const id = item.annotation?.id;
		if (!id) {
			out.push(item);
			continue;
		}
		if (seen.has(id)) continue;
		seen.add(id);
		out.push(item);
	}
	return out;
}

/** Canonical path: `papers/<id>/marks/annotations.json`. */
function annotationsPath(paperAbsPath: string): string {
	return joinVaultPath(
		joinVaultPath(paperAbsPath, MARKS_FOLDER),
		ANNOTATIONS_FILE,
	);
}

/** Pre-move path: `papers/<id>/annotations.json` (paper root). */
function legacyAnnotationsPath(paperAbsPath: string): string {
	return joinVaultPath(paperAbsPath, ANNOTATIONS_FILE);
}

async function removeFileIfExists(path: string): Promise<void> {
	try {
		const { remove } = await import("@tauri-apps/plugin-fs");
		await remove(path);
	} catch {
		// missing is fine
	}
}

function memberAnnotationPath(
	paperAbsPath: string,
	userId: string,
	id: string,
): string {
	return joinVaultPath(
		joinVaultPath(joinVaultPath(paperAbsPath, MARKS_FOLDER), userId),
		`${id}.json`,
	);
}

function safeMarkId(id: string): boolean {
	return /^[A-Za-z0-9_-]{1,128}$/.test(id);
}

async function readTransferItems(
	path: string,
): Promise<AnnotationTransferItem[]> {
	try {
		const parsed = JSON.parse(await readVaultFile(path)) as unknown;
		if (Array.isArray(parsed)) return parsed as AnnotationTransferItem[];
		const item = parsed as AnnotationTransferItem;
		return item?.annotation?.id ? [item] : [];
	} catch {
		return [];
	}
}

async function loadSharedAnnotationItems(
	paperAbsPath: string,
	userId: string | null,
): Promise<AnnotationTransferItem[]> {
	const ownerMap = new Map<string, string>();
	const itemsById = new Map<string, AnnotationTransferItem>();
	// The signed-in member's file wins, body and owner together. A later file
	// must not keep the first body while labeling the id as ours.
	const take = (item: AnnotationTransferItem, owner: string) => {
		const id = item.annotation?.id;
		if (!id) return;
		const current = ownerMap.get(id);
		if (current === userId) return;
		if (owner === userId || current === undefined) {
			itemsById.set(id, item);
			ownerMap.set(id, owner);
		}
	};

	for (const file of await listMarkJsonFiles(paperAbsPath)) {
		const owner =
			file.owner && userId && file.owner === userId
				? userId
				: (file.owner ?? AGGREGATE_OWNER);
		for (const item of await readTransferItems(file.path)) {
			take(item, owner);
		}
	}
	if (userId) {
		for (const path of [
			annotationsPath(paperAbsPath),
			legacyAnnotationsPath(paperAbsPath),
		]) {
			const aggregate = await readTransferItems(path);
			let copied = true;
			for (const item of aggregate) {
				const id = item.annotation?.id;
				if (!id || !safeMarkId(id) || ownerMap.has(id)) continue;
				try {
					const dest = memberAnnotationPath(paperAbsPath, userId, id);
					markSelfWrite(dest);
					await writeVaultFile(dest, `${JSON.stringify(item, null, 2)}\n`);
					take(item, userId);
				} catch {
					copied = false;
				}
			}
			if (copied && aggregate.length > 0) {
				await removeFileIfExists(path);
			}
		}
	}
	ownersByPaper.set(paperAbsPath, ownerMap);
	return dedupeAnnotationItems([...itemsById.values()]);
}

async function saveSharedAnnotationItems(
	paperAbsPath: string,
	items: AnnotationTransferItem[],
	userId: string,
): Promise<void> {
	const ownerMap = ownersByPaper.get(paperAbsPath) ?? new Map<string, string>();
	const exported = new Set<string>();
	for (const item of items) {
		const id = item.annotation?.id;
		if (!id) continue;
		exported.add(id);
		const owner = ownerMap.get(id);
		if (owner && owner !== userId) continue;
		const path = memberAnnotationPath(paperAbsPath, userId, id);
		markSelfWrite(path);
		await writeVaultFile(path, `${JSON.stringify(item, null, 2)}\n`);
		ownerMap.set(id, userId);
	}
	for (const file of await listMarkJsonFiles(paperAbsPath)) {
		if (file.owner !== userId || exported.has(file.id)) continue;
		const stored = await readTransferItems(file.path);
		if (!stored.some((item) => item.annotation?.id === file.id)) continue;
		markSelfWrite(file.path);
		try {
			await removeVaultPath(file.path);
		} catch {
			// already gone
		}
		ownerMap.delete(file.id);
	}
	ownersByPaper.set(paperAbsPath, ownerMap);
}

export async function loadAnnotationItems(
	paperAbsPath: string,
): Promise<AnnotationTransferItem[]> {
	if (!paperAbsPath) return [];
	if (!isTauri()) {
		return dedupeAnnotationItems(memoryStore.get(paperAbsPath) ?? []);
	}
	const layout = await memberLayout(paperAbsPath);
	if (layout.shared) {
		return loadSharedAnnotationItems(paperAbsPath, layout.userId);
	}

	// Prefer marks/; fall back to paper-root and migrate once.
	try {
		const raw = await readVaultFile(annotationsPath(paperAbsPath));
		const parsed = JSON.parse(raw) as unknown;
		return Array.isArray(parsed)
			? dedupeAnnotationItems(parsed as AnnotationTransferItem[])
			: [];
	} catch {
		// continue to legacy
	}

	try {
		const raw = await readVaultFile(legacyAnnotationsPath(paperAbsPath));
		const parsed = JSON.parse(raw) as unknown;
		if (!Array.isArray(parsed)) return [];
		const items = dedupeAnnotationItems(parsed as AnnotationTransferItem[]);
		if (items.length) {
			await saveAnnotationItems(paperAbsPath, items);
		} else {
			// Empty file still belongs under marks/ once discovered.
			const path = annotationsPath(paperAbsPath);
			markSelfWrite(path);
			await writeVaultFile(path, `${JSON.stringify([], null, 2)}\n`);
		}
		await removeFileIfExists(legacyAnnotationsPath(paperAbsPath));
		return items;
	} catch {
		return [];
	}
}

export async function saveAnnotationItems(
	paperAbsPath: string,
	items: AnnotationTransferItem[],
): Promise<void> {
	if (!paperAbsPath) return;
	const next = dedupeAnnotationItems(items);
	if (!isTauri()) {
		memoryStore.set(paperAbsPath, next);
		return;
	}
	const layout = await memberLayout(paperAbsPath);
	if (layout.shared) {
		if (!layout.userId) return;
		await saveSharedAnnotationItems(paperAbsPath, next, layout.userId);
		return;
	}
	const path = annotationsPath(paperAbsPath);
	markSelfWrite(path);
	await writeVaultFile(path, `${JSON.stringify(next, null, 2)}\n`);
	// Drop legacy root copy if present so marks/ is the only location.
	await removeFileIfExists(legacyAnnotationsPath(paperAbsPath));
}

export async function hasAnnotationsFile(
	paperAbsPath: string,
): Promise<boolean> {
	if (!paperAbsPath) return false;
	if (!isTauri()) return memoryStore.has(paperAbsPath);
	const layout = await memberLayout(paperAbsPath);
	if (layout.shared) {
		for (const file of await listMarkJsonFiles(paperAbsPath)) {
			const stored = await readTransferItems(file.path);
			if (stored.length) return true;
		}
	}
	for (const path of [
		annotationsPath(paperAbsPath),
		legacyAnnotationsPath(paperAbsPath),
	]) {
		try {
			await readVaultFile(path);
			return true;
		} catch {
			// try next
		}
	}
	return false;
}

export function isHighlightObject(
	obj: PdfAnnotationObject,
): obj is PdfHighlightAnnoObject {
	return obj.type === PdfAnnotationSubtype.HIGHLIGHT;
}

/**
 * EmbedPDF keeps a deleted annotation in `byUid` until an async commit purges
 * it, and `getAnnotations()` / `getAnnotationById()` still return it as
 * `commitState: "deleted"` in the meantime (or forever if the commit was
 * dropped). Rendering, hit testing and editors must treat those as gone.
 */
export function isLiveAnnotation(
	annotation: Pick<TrackedAnnotation, "commitState">,
): boolean {
	return annotation.commitState !== "deleted";
}

/** App-specific metadata round-tripped through the annotation `custom` field. */
export type HighlightCustom = {
	app?: string;
	paletteKey?: HighlightColor;
	quote?: string;
	/**
	 * Page-fraction boxes of the translated selection on this annotation's
	 * page. Absent for a highlight made on the English text layer.
	 */
	translatedRects?: PdfHighlight["translatedRects"];
};

export function highlightColorOf(obj: PdfHighlightAnnoObject): HighlightColor {
	const custom = (obj.custom ?? {}) as HighlightCustom;
	if (custom.paletteKey) return custom.paletteKey;
	return highlightColorFromHex(obj.strokeColor ?? obj.color);
}

export function highlightQuoteOf(obj: PdfHighlightAnnoObject): string {
	const custom = (obj.custom ?? {}) as HighlightCustom;
	return custom.quote?.trim() ?? "";
}

/** Build the PdfHighlight view model that the annotations panel + handle use. */
export function highlightViewFromObject(
	obj: PdfHighlightAnnoObject,
	paperPath: string,
): PdfHighlight {
	const iso = (d?: Date) =>
		d instanceof Date ? d.toISOString() : new Date().toISOString();
	const comment = obj.contents?.trim();
	const view: PdfHighlight = {
		version: 1,
		kind: "highlight",
		id: obj.id,
		paperPath,
		createdAt: iso(obj.created),
		updatedAt: iso(obj.modified ?? obj.created),
		page: obj.pageIndex + 1,
		rects: [],
		quote: highlightQuoteOf(obj),
		color: highlightColorOf(obj),
	};
	if (comment) view.comment = comment;
	const translated = sanitizeTranslatedRects(
		(obj.custom as HighlightCustom | undefined)?.translatedRects,
	);
	if (translated.length > 0) view.translatedRects = translated;
	return view;
}
