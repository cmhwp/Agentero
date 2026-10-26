/**
 * One virtualized paper row for the library table: cells rendered from
 * COLUMN_META plus the row context menu (open / download / edit / reveal /
 * delete / add-to-chat).
 *
 * Single-click opens the paper; double-click a copyable cell copies that field.
 *
 * Memoized — the parent must keep `ctx` (t + onCellCopy) and `measureRef`
 * reference-stable or every scroll re-renders all visible rows.
 */
import {
	BookOpen,
	Download,
	FileText,
	FolderOpen,
	MessageSquarePlus,
	Pencil,
	RefreshCw,
	Trash2,
} from "lucide-react";
import type { MouseEvent } from "react";
import { Fragment, memo } from "react";
import { COLUMN_META } from "@/components/library/library-columns";
import {
	type CellCtx,
	isMissingLocalPdf,
	type PaperRow,
} from "@/components/library/library-row-utils";
import {
	ContextMenu,
	ContextMenuContent,
	ContextMenuItem,
	ContextMenuSeparator,
	ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { presentAgentComposer } from "@/lib/agent/composer-focus";
import { broadcastAgentAttachContext } from "@/lib/agent/context-attach";
import { notifyError } from "@/lib/core/notify";
import type { PaperMetadata } from "@/lib/paper";
import { downloadLibraryPaper } from "@/lib/paper/library-actions";
import { setEditMetaDraft } from "@/lib/paper/library-store";
import type { ReadingHeatmap } from "@/lib/paper/reading-heatmap";
import type { LibraryColumnPref } from "@/lib/settings";
import type { MenuContext } from "@/lib/shell/commands/menu-contributions";
import {
	activateMenuContribution,
	visibleMenuContributions,
} from "@/lib/shell/commands/menu-contributions";
import { trashPathsAndNotify } from "@/lib/vault/actions";
import { revealInFileManager, revealInOsLabelKey } from "@/lib/vault/reveal";

/**
 * Reads the contribution list when the menu content mounts, not when the
 * memoized row last rendered. An empty list renders nothing, including no
 * separator.
 */
function LibraryRowMenuContributions({
	vaultPath,
	relPath,
	paperId,
}: {
	vaultPath: string | null;
	relPath: string | undefined;
	paperId: string | undefined;
}) {
	const menuContext: MenuContext = { vaultPath };
	if (relPath) menuContext.relPath = relPath;
	if (paperId) menuContext.paperId = paperId;
	const items = visibleMenuContributions("library.row", menuContext);
	if (items.length === 0) return null;
	return (
		<>
			<ContextMenuSeparator />
			{items.map((item) => (
				<ContextMenuItem
					key={item.id}
					onSelect={() => {
						activateMenuContribution(item, menuContext);
					}}
				>
					{item.title}
				</ContextMenuItem>
			))}
		</>
	);
}

type LibraryPaperRowProps = {
	/** Virtualizer index (`data-index` for measureElement). */
	index: number;
	row: PaperRow;
	heat: ReadingHeatmap | undefined;
	visibleColumns: LibraryColumnPref[];
	/** Stable `t` + `onCellCopy` pair shared by all rows (memo friendly). */
	ctx: Pick<CellCtx, "t" | "onCellCopy">;
	paperAbsPath: string | null;
	/** Vault root the library view already has. Null when it does not. */
	vaultPath?: string | null;
	canEditMeta: boolean;
	onOpenPaper: (paper: PaperMetadata) => void;
	onRowClick: (
		e: MouseEvent<HTMLTableRowElement>,
		paper: PaperMetadata,
	) => void;
	onRefreshMetadata?: (paper: PaperMetadata) => void;
	onCreateReviewCard?: (paperAbsPath: string, paper: PaperMetadata) => void;
	/** Preview the row in the left Paper Info panel while hovered. */
	onHoverPaper?: (relPath: string | null) => void;
	/** rowVirtualizer.measureElement — attached to the `<tr>`. */
	measureRef: (element: Element | null) => void;
};

export const LibraryPaperRow = memo(function LibraryPaperRow({
	index,
	row,
	heat,
	visibleColumns,
	ctx,
	paperAbsPath,
	vaultPath = null,
	canEditMeta,
	onOpenPaper,
	onRowClick,
	onRefreshMetadata,
	onCreateReviewCard,
	onHoverPaper,
	measureRef,
}: LibraryPaperRowProps) {
	const p = row.paper;
	const cellCtx: CellCtx = {
		t: ctx.t,
		onCellCopy: ctx.onCellCopy,
		heat,
		tags: row.tags,
	};
	return (
		<ContextMenu>
			<ContextMenuTrigger asChild>
				<tr
					data-index={index}
					ref={measureRef}
					className="border-border/40 border-b transition-colors duration-100 hover:bg-accent/70 active:bg-accent"
					onClick={(e) => onRowClick(e, p)}
					onMouseEnter={() => onHoverPaper?.(p.path ?? null)}
				>
					{visibleColumns.map((col) => (
						<Fragment key={col.key}>
							{COLUMN_META[col.key].render(p, cellCtx)}
						</Fragment>
					))}
				</tr>
			</ContextMenuTrigger>
			<ContextMenuContent className="w-44">
				<ContextMenuItem onSelect={() => onOpenPaper(p)}>
					<BookOpen className="size-3.5" />
					{ctx.t("papersLibrary.rowOpen")}
				</ContextMenuItem>
				{canEditMeta && isMissingLocalPdf(p) ? (
					<ContextMenuItem onSelect={() => void downloadLibraryPaper(p)}>
						<Download className="size-3.5" />
						{ctx.t("papersLibrary.rowDownloadPdf")}
					</ContextMenuItem>
				) : null}
				{canEditMeta ? (
					<ContextMenuItem onSelect={() => setEditMetaDraft(p)}>
						<Pencil className="size-3.5" />
						{ctx.t("papersLibrary.rowEditMeta")}
					</ContextMenuItem>
				) : null}
				{canEditMeta && paperAbsPath && onCreateReviewCard ? (
					<ContextMenuItem onSelect={() => onCreateReviewCard(paperAbsPath, p)}>
						<FileText className="size-3.5" />
						{ctx.t("papersLibrary.createReviewCard")}
					</ContextMenuItem>
				) : null}
				{canEditMeta &&
				onRefreshMetadata &&
				(p.doi?.trim() || p.arxiv_id?.trim() || p.title?.trim()) ? (
					<ContextMenuItem onSelect={() => void onRefreshMetadata?.(p)}>
						<RefreshCw className="size-3.5" />
						{ctx.t("papersLibrary.rowRefreshMeta")}
					</ContextMenuItem>
				) : null}
				{paperAbsPath ? (
					<>
						<ContextMenuSeparator />
						<ContextMenuItem
							onSelect={() => {
								broadcastAgentAttachContext([paperAbsPath]);
								presentAgentComposer();
							}}
						>
							<MessageSquarePlus className="size-3.5" />
							{ctx.t("fileTree.addToChat")}
						</ContextMenuItem>
						{canEditMeta ? (
							<ContextMenuItem
								onSelect={() => {
									void revealInFileManager(paperAbsPath).catch(() =>
										notifyError(ctx.t("fileTree.revealFailed")),
									);
								}}
							>
								<FolderOpen className="size-3.5" />
								{ctx.t(revealInOsLabelKey())}
							</ContextMenuItem>
						) : null}
						<ContextMenuItem
							className="text-destructive focus:text-destructive"
							onSelect={() => void trashPathsAndNotify([paperAbsPath])}
						>
							<Trash2 className="size-3.5" />
							{ctx.t("fileTree.delete")}
						</ContextMenuItem>
					</>
				) : null}
				<LibraryRowMenuContributions
					vaultPath={vaultPath}
					relPath={p.path || undefined}
					paperId={p.id || undefined}
				/>
			</ContextMenuContent>
		</ContextMenu>
	);
});
