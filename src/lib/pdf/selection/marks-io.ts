/**
 * Unified on-disk layout for PDF selection marks:
 *
 *   papers/<id>/marks/<id>.json          # kind: ask | highlight | translate | visual
 *   papers/<id>/marks/annotations.json   # EmbedPDF highlight/批注 transfer blob
 *
 * Per-mark files are pretty JSON with required `kind`.
 * Legacy visual marks used kind `agent-trace` (still readable).
 * `annotations.json` is the aggregate EmbedPDF annotation store (not a mark).
 */
import { readDir } from "@tauri-apps/plugin-fs";

import { isTauri } from "@/lib/core/tauri";
import { memberLayout } from "@/lib/library/member-layout";
import {
	joinVaultPath,
	normalizePathKey,
	readVaultFile,
	removeVaultPath,
	writeVaultFile,
} from "@/lib/vault";

export const MARKS_FOLDER = "marks";

/** Aggregate EmbedPDF annotations file name under `marks/` (not a per-id mark). */
export const ANNOTATIONS_JSON = "annotations.json";

export function marksDir(paperAbsPath: string): string {
	return joinVaultPath(paperAbsPath, MARKS_FOLDER);
}

export function markPath(paperAbsPath: string, id: string): string {
	return joinVaultPath(marksDir(paperAbsPath), `${id}.json`);
}

export type MarkJsonFile = {
	id: string;
	path: string;
	/** Member directory name, or null for a flat `marks/<id>.json`. */
	owner: string | null;
};

function markIdFromName(name: string): string | null {
	if (!name.endsWith(".json") || name === ANNOTATIONS_JSON) return null;
	return name.slice(0, -".json".length);
}

/** Flat mark files plus one level of `marks/{userId}/`. Skips `annotations.json`. */
export async function listMarkJsonFiles(
	paperAbsPath: string,
): Promise<MarkJsonFile[]> {
	if (!paperAbsPath || !isTauri()) return [];
	const dir = marksDir(paperAbsPath);
	let entries: Awaited<ReturnType<typeof readDir>> = [];
	try {
		entries = await readDir(dir);
	} catch {
		return [];
	}
	const out: MarkJsonFile[] = [];
	for (const entry of entries) {
		const name = entry.name;
		if (!name) continue;
		if (entry.isDirectory) {
			let children: Awaited<ReturnType<typeof readDir>> = [];
			try {
				children = await readDir(joinVaultPath(dir, name));
			} catch {
				continue;
			}
			for (const child of children) {
				const childName = child.name;
				if (!childName || child.isDirectory) continue;
				const id = markIdFromName(childName);
				if (!id) continue;
				out.push({
					id,
					owner: name,
					path: joinVaultPath(joinVaultPath(dir, name), childName),
				});
			}
			continue;
		}
		const id = markIdFromName(name);
		if (!id) continue;
		out.push({ id, owner: null, path: joinVaultPath(dir, name) });
	}
	return out;
}

async function sharedMarkPath(
	paperAbsPath: string,
	id: string,
): Promise<string | null> {
	const layout = await memberLayout(paperAbsPath);
	if (!layout.shared) return null;
	if (!layout.userId) {
		throw new Error("Sign in to add marks in a shared library");
	}
	return joinVaultPath(
		joinVaultPath(marksDir(paperAbsPath), layout.userId),
		`${id}.json`,
	);
}

/** Sentinel for files that failed to read/parse (never a real JSON value). */
const CORRUPT_MARK = Symbol("marks.corrupt");

/** Read + JSON.parse every per-id `*.json` under `marks/` (skip aggregate + corrupt). */
export async function listMarkRaw(paperAbsPath: string): Promise<unknown[]> {
	if (!paperAbsPath || !isTauri()) return [];
	const files = await listMarkJsonFiles(paperAbsPath);
	const layout = await memberLayout(paperAbsPath);
	const byId = new Map<string, (typeof files)[number]>();
	for (const file of files) {
		const current = byId.get(file.id);
		if (!current || (layout.userId && file.owner === layout.userId)) {
			byId.set(file.id, file);
		}
	}
	const loaded = await Promise.all(
		[...byId.values()].map(async (file): Promise<unknown> => {
			try {
				const raw = await readVaultFile(file.path);
				return JSON.parse(raw) as unknown;
			} catch {
				return CORRUPT_MARK;
			}
		}),
	);
	return loaded.filter((mark) => mark !== CORRUPT_MARK);
}

export async function readMarkRaw(
	paperAbsPath: string,
	id: string,
): Promise<unknown | null> {
	if (!paperAbsPath || !id || !isTauri()) return null;
	try {
		const shared = await sharedMarkPath(paperAbsPath, id).catch(() => null);
		const raw = await readVaultFile(shared ?? markPath(paperAbsPath, id));
		return JSON.parse(raw) as unknown;
	} catch {
		try {
			const raw = await readVaultFile(markPath(paperAbsPath, id));
			return JSON.parse(raw) as unknown;
		} catch {
			return null;
		}
	}
}

/**
 * Self-write echo suppression: watcher events caused by this app's own
 * `marks/` writes are pure echo for the marks refresh (the writer already
 * updated in-memory state), so the marks watcher skips events whose paths are
 * all recent self-writes. Writers register the path here; the TTL covers the
 * filesystem-watcher latency between write and event.
 */
const SELF_WRITE_TTL_MS = 3000;
const selfWrites = new Map<string, number>();

/** Register a `marks/` path this app is about to write (echo suppression). */
export function markSelfWrite(path: string): void {
	const now = Date.now();
	selfWrites.set(normalizePathKey(path), now);
	for (const [key, at] of selfWrites) {
		if (now - at > SELF_WRITE_TTL_MS) selfWrites.delete(key);
	}
}

/** True when `path` was written by this app within the echo TTL. */
export function isRecentSelfWrite(path: string): boolean {
	const at = selfWrites.get(normalizePathKey(path));
	return at !== undefined && Date.now() - at <= SELF_WRITE_TTL_MS;
}

export async function writeMarkFile(
	paperAbsPath: string,
	id: string,
	payload: unknown,
): Promise<void> {
	const shared = await sharedMarkPath(paperAbsPath, id);
	const path = shared ?? markPath(paperAbsPath, id);
	markSelfWrite(path);
	await writeVaultFile(path, `${JSON.stringify(payload, null, 2)}\n`);
	if (shared) {
		const flat = markPath(paperAbsPath, id);
		markSelfWrite(flat);
		try {
			await removeVaultPath(flat);
		} catch {
			// no legacy flat file
		}
	}
}

export async function deleteMarkFile(
	paperAbsPath: string,
	id: string,
): Promise<void> {
	if (!isTauri() || !paperAbsPath || !id) return;
	const layout = await memberLayout(paperAbsPath);
	if (layout.shared) {
		if (!layout.userId) {
			throw new Error("Sign in to add marks in a shared library");
		}
		const files = await listMarkJsonFiles(paperAbsPath);
		const mine = files.find(
			(file) => file.id === id && file.owner === layout.userId,
		);
		const flat = files.find((file) => file.id === id && file.owner === null);
		const others = files.some(
			(file) => file.id === id && file.owner && file.owner !== layout.userId,
		);
		for (const file of [mine, others ? undefined : flat]) {
			if (!file) continue;
			markSelfWrite(file.path);
			try {
				await removeVaultPath(file.path);
			} catch {
				// missing is fine
			}
		}
		return;
	}
	markSelfWrite(markPath(paperAbsPath, id));
	try {
		await removeVaultPath(markPath(paperAbsPath, id));
	} catch {
		// missing is fine
	}
}
