import {
	Bot,
	ChevronRight,
	Database,
	Download,
	FolderKanban,
	Globe,
	Library,
	Link2,
	Loader2,
	Notebook,
	NotebookPen,
	Play,
	ScrollText,
	Sparkles,
	Trash2,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import {
	FileTreeActions,
	FileTreeDisclosureIcon,
	FileTreeFile,
	FileTreeFolderRow,
	FileTreeIcon,
	FileTreeName,
	useFileTree,
} from "@/components/ai-elements/file-tree";
import { PLAZA_SOURCE_ICONS } from "@/components/plaza/source-icons";
import { Button } from "@/components/ui/button";
import { MathText } from "@/components/ui/math-text";
import { Skeleton } from "@/components/ui/skeleton";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import { contextPathIcon } from "@/lib/agent/context-path-icon";
import { cn } from "@/lib/core/utils";
import { isPapersRoot } from "@/lib/paper";
import { TRASH_VIRTUAL_PATH } from "@/lib/paper/api";
import {
	PLAZA_VIRTUAL_PATH,
	type PlazaSource,
	plazaSourceLabel,
} from "@/lib/plaza";
import type { FileNode } from "@/lib/vault";
import {
	isVaultAgentRoot,
	isVaultSkillDirectory,
	type SkillTreeLabel,
	skillDescriptionTooltip,
} from "@/lib/vault/agent-tree";
import { normalizePathKey } from "@/lib/vault/path";
import type { TexCompileActions as TexCompileHookActions } from "./hooks/use-tex-compile";
import {
	DOWNLOAD_REASON_KEYS,
	VAULT_ROOT_LABEL_KEY,
	type VaultRootFace,
	vaultRootFace,
} from "./tree-helpers";

type PaperTreeRowProps = {
	node: FileNode;
	isCut: boolean;
	label: string;
	downloadReasons: Array<keyof typeof DOWNLOAD_REASON_KEYS>;
	isDownloading: boolean;
	isReading: boolean;
	rowBusy: boolean;
	expandable: boolean;
	expanded: boolean;
	onDownload?: () => void;
	onRead?: () => void;
};

export function PaperTreeRow({
	node,
	isCut,
	label,
	downloadReasons,
	isDownloading,
	isReading,
	rowBusy,
	expandable,
	expanded,
	onDownload,
	onRead,
}: PaperTreeRowProps) {
	const { t } = useTranslation("sidebar");
	const { togglePath } = useFileTree();
	const expandLabel = expanded
		? t("fileTree.collapseChildren")
		: t("fileTree.expandChildren");
	const showDownload = Boolean(onDownload) && downloadReasons.length > 0;
	const showRead = Boolean(onRead) && !showDownload;
	const reasonTip = downloadReasons.length
		? downloadReasons.map((r) => t(DOWNLOAD_REASON_KEYS[r])).join(" · ")
		: t("fileTree.downloadAssets");
	const showActions = showDownload || showRead;
	return (
		<FileTreeFile
			path={node.path}
			name={label}
			className={cn(isCut && "opacity-50")}
		>
			{expandable ? (
				<button
					type="button"
					aria-expanded={expanded}
					aria-label={expandLabel}
					className={cn(
						"group/attachment relative flex size-4 shrink-0 items-center justify-center",
						"text-muted-foreground focus-visible:outline-none",
					)}
					onClick={(e) => {
						e.stopPropagation();
						e.preventDefault();
						togglePath(node.path);
					}}
					onPointerDown={(e) => e.stopPropagation()}
					onKeyDown={(e) => e.stopPropagation()}
				>
					<ScrollText
						className="size-4 transition-opacity group-hover/attachment:opacity-0 group-focus-visible/attachment:opacity-0"
						aria-hidden
					/>
					<ChevronRight
						className={cn(
							"pointer-events-none absolute size-4 opacity-0 transition-[opacity,transform] group-hover/attachment:opacity-100 group-focus-visible/attachment:opacity-100",
							expanded && "rotate-90",
						)}
						aria-hidden
					/>
				</button>
			) : (
				<ScrollText
					className="size-4 shrink-0 text-muted-foreground"
					aria-hidden
				/>
			)}
			<FileTreeName className="min-w-0 flex-1 truncate" title={label}>
				<MathText text={label} />
			</FileTreeName>
			{showActions ? (
				<FileTreeActions
					className="shrink-0"
					onClick={(e) => {
						e.stopPropagation();
					}}
					onKeyDown={(e) => e.stopPropagation()}
				>
					{showDownload ? (
						<Tooltip disableHoverableContent>
							<TooltipTrigger asChild>
								<Button
									type="button"
									variant="ghost"
									size="icon-xs"
									className="size-5"
									aria-label={reasonTip}
									disabled={rowBusy}
									onClick={(e) => {
										e.stopPropagation();
										onDownload?.();
									}}
								>
									{isDownloading ? (
										<Loader2 className="size-3.5 animate-spin" />
									) : (
										<Download className="size-3.5" />
									)}
								</Button>
							</TooltipTrigger>
							<TooltipContent
								side="right"
								className="max-w-xs select-none cursor-default flex-col items-start gap-0"
							>
								<p className="font-medium">{t("fileTree.downloadAssets")}</p>
								<ul className="mt-1 list-disc space-y-0.5 pl-3 text-xs opacity-90">
									{downloadReasons.map((r) => (
										<li key={r}>{t(DOWNLOAD_REASON_KEYS[r])}</li>
									))}
								</ul>
							</TooltipContent>
						</Tooltip>
					) : null}
					{showRead ? (
						<Tooltip disableHoverableContent>
							<TooltipTrigger asChild>
								<Button
									type="button"
									variant="ghost"
									size="icon-xs"
									className="size-5"
									data-read-paper
									aria-label={t("fileTree.readPaper")}
									disabled={rowBusy}
									onClick={(e) => {
										e.stopPropagation();
										onRead?.();
									}}
								>
									{isReading ? (
										<Loader2 className="size-3.5 animate-spin" />
									) : (
										<NotebookPen className="size-3.5" />
									)}
								</Button>
							</TooltipTrigger>
							<TooltipContent
								side="right"
								className="max-w-xs select-none cursor-default"
							>
								<p className="font-medium">{t("fileTree.readPaper")}</p>
							</TooltipContent>
						</Tooltip>
					) : null}
				</FileTreeActions>
			) : null}
		</FileTreeFile>
	);
}

type NodeTreeRowProps = {
	node: FileNode;
	isCut: boolean;
	pendingLoad: boolean;
	expanded: boolean;
	texCompile?: TexCompileHookActions;
	vaultPath?: string | null;
	skillLabels?: ReadonlyMap<string, SkillTreeLabel>;
};

export function NodeTreeRow({
	node,
	isCut,
	pendingLoad,
	expanded,
	texCompile,
	vaultPath,
	skillLabels,
}: NodeTreeRowProps) {
	const { t } = useTranslation("sidebar");
	if (
		node.kind === "directory" &&
		isVaultSkillDirectory(vaultPath, node.path)
	) {
		const label = skillLabels?.get(normalizePathKey(node.path));
		return (
			<SkillTreeRow
				node={node}
				isCut={isCut}
				label={label?.name || node.name}
				description={skillDescriptionTooltip(label?.description ?? "")}
				expanded={expanded}
				pendingLoad={pendingLoad}
			/>
		);
	}
	if (node.kind === "directory") {
		// `papers/` doubles as the library entry: library icon + title.
		// Vault-root notes/thesis/data, and `.agents`, get an icon + label.
		// Disk names stay unchanged.
		const isLibraryRoot = isPapersRoot(node.path);
		const isAgentRoot = isVaultAgentRoot(vaultPath, node.path);
		const rootFace =
			isLibraryRoot || isAgentRoot ? null : vaultRootFace(vaultPath, node.path);
		return (
			<div
				className={cn(
					"relative flex w-full items-center",
					isCut && "opacity-50",
				)}
				{...(isLibraryRoot ? { "data-library-row": "" } : {})}
				{...(isAgentRoot ? { "data-vault-root": "agents" } : {})}
				{...(rootFace ? { "data-vault-root": rootFace } : {})}
			>
				<div className="min-w-0 flex-1">
					<FileTreeFolderRow
						path={node.path}
						name={
							isLibraryRoot
								? t("papersLibrary.title")
								: isAgentRoot
									? t("fileTree.vaultRoot.agents")
									: rootFace
										? t(VAULT_ROOT_LABEL_KEY[rootFace])
										: node.name
						}
						icon={
							isLibraryRoot ? (
								<Library className="size-4 text-muted-foreground" />
							) : isAgentRoot ? (
								<Bot className="size-4 text-muted-foreground" aria-hidden />
							) : rootFace ? (
								<VaultRootIcon face={rootFace} />
							) : undefined
						}
					/>
				</div>
				{pendingLoad && expanded ? (
					<Loader2
						className="pointer-events-none absolute right-2 size-3.5 shrink-0 animate-spin text-muted-foreground"
						aria-hidden
					/>
				) : null}
			</div>
		);
	}

	const Icon = contextPathIcon(node.name);
	const isTex = texCompile?.isTexFile(node.path) ?? false;
	const isCompiling = texCompile?.compilingPath === node.path;

	return (
		<FileTreeFile
			path={node.path}
			name={node.name}
			className={cn(isCut && "opacity-50")}
		>
			<FileTreeIcon>
				<Icon className="size-4 text-muted-foreground" />
			</FileTreeIcon>
			<FileTreeName className="min-w-0 flex-1 truncate" title={node.name}>
				{node.name}
			</FileTreeName>
			{isTex && texCompile ? (
				<TexCompileActions
					actions={texCompile}
					texPath={node.path}
					vaultPath={vaultPath ?? null}
					isCompiling={isCompiling}
				/>
			) : null}
		</FileTreeFile>
	);
}

function SkillTreeRow({
	node,
	isCut,
	label,
	description,
	expanded,
	pendingLoad,
}: {
	node: FileNode;
	isCut: boolean;
	label: string;
	description: string;
	expanded: boolean;
	pendingLoad: boolean;
}) {
	const { t } = useTranslation("sidebar");
	const { togglePath } = useFileTree();
	const expandable =
		Boolean(node.childrenPending) || (node.children?.length ?? 0) > 0;
	const expandLabel = expanded
		? t("fileTree.collapseChildren")
		: t("fileTree.expandChildren");
	const tip = description || label;
	return (
		<div
			className={cn("relative flex w-full items-center", isCut && "opacity-50")}
		>
			<FileTreeFile path={node.path} name={label} className="min-w-0 flex-1">
				{expandable ? (
					<button
						type="button"
						aria-expanded={expanded}
						aria-label={expandLabel}
						className="group/skill relative flex size-4 shrink-0 items-center justify-center text-muted-foreground focus-visible:outline-none"
						onClick={(e) => {
							e.stopPropagation();
							e.preventDefault();
							togglePath(node.path);
						}}
						onPointerDown={(e) => e.stopPropagation()}
						onKeyDown={(e) => e.stopPropagation()}
					>
						<Sparkles
							className="size-4 transition-opacity group-hover/skill:opacity-0 group-focus-visible/skill:opacity-0"
							aria-hidden
						/>
						<ChevronRight
							className={cn(
								"pointer-events-none absolute size-4 opacity-0 transition-[opacity,transform] group-hover/skill:opacity-100 group-focus-visible/skill:opacity-100",
								expanded && "rotate-90",
							)}
							aria-hidden
						/>
					</button>
				) : (
					<Sparkles
						className="size-4 shrink-0 text-muted-foreground"
						aria-hidden
					/>
				)}
				<FileTreeName className="min-w-0 flex-1 truncate" title={tip}>
					{label}
				</FileTreeName>
			</FileTreeFile>
			{pendingLoad && expanded ? (
				<Loader2
					className="pointer-events-none absolute right-2 size-3.5 shrink-0 animate-spin text-muted-foreground"
					aria-hidden
				/>
			) : null}
		</div>
	);
}

function VaultRootIcon({ face }: { face: VaultRootFace }) {
	if (face === "projects") {
		return <FolderKanban className="size-4 text-violet-500" aria-hidden />;
	}
	const Icon =
		face === "notes" ? Notebook : face === "thesis" ? ScrollText : Database;
	return <Icon className="size-4 text-muted-foreground" aria-hidden />;
}

/** Play button for a .tex row. Engine, live preview, and clean live in the LaTeX panel. */
function TexCompileActions({
	actions,
	texPath,
	vaultPath,
	isCompiling,
}: {
	actions: TexCompileHookActions;
	texPath: string;
	vaultPath: string | null;
	isCompiling: boolean;
}) {
	const { t } = useTranslation("sidebar");
	const hasEngines = !actions.enginesLoading && actions.engines.length > 0;

	if (isCompiling) {
		return (
			<FileTreeActions
				className="shrink-0"
				onClick={(e) => e.stopPropagation()}
				onKeyDown={(e) => e.stopPropagation()}
			>
				<Loader2
					className="size-3.5 animate-spin text-muted-foreground"
					aria-label={t("fileTree.compileTex")}
				/>
			</FileTreeActions>
		);
	}

	return (
		<FileTreeActions
			className="shrink-0"
			onClick={(e) => e.stopPropagation()}
			onKeyDown={(e) => e.stopPropagation()}
		>
			<Tooltip disableHoverableContent>
				<TooltipTrigger asChild>
					<Button
						type="button"
						variant="ghost"
						size="icon-xs"
						className="size-5 bg-transparent hover:bg-transparent active:bg-transparent dark:hover:bg-transparent dark:active:bg-transparent group-aria-selected:focus-visible:border-sidebar-primary-foreground/70 group-aria-selected:focus-visible:ring-sidebar-primary-foreground/50"
						data-tex-compile=""
						aria-label={t("fileTree.compileTex")}
						disabled={!hasEngines || !vaultPath}
						onClick={(e) => {
							e.stopPropagation();
							if (vaultPath && hasEngines) {
								void actions.compileTex(texPath, vaultPath);
							}
						}}
					>
						<Play className="size-3.5" />
					</Button>
				</TooltipTrigger>
				<TooltipContent side="right" className="select-none cursor-default">
					{t("fileTree.compileTex")}
				</TooltipContent>
			</Tooltip>
		</FileTreeActions>
	);
}

export function TrashRow() {
	const { t } = useTranslation("sidebar");
	return (
		<FileTreeFile path={TRASH_VIRTUAL_PATH} name={t("recycleBin.title")}>
			<FileTreeIcon>
				<Trash2 className="size-4 text-muted-foreground" />
			</FileTreeIcon>
			<FileTreeName className="min-w-0 flex-1 truncate">
				{t("recycleBin.title")}
			</FileTreeName>
		</FileTreeFile>
	);
}

/** 广场 parent row — collapsible, with the discovery sources as children. */
export function PlazaRow({ expanded }: { expanded: boolean }) {
	const { t } = useTranslation("sidebar");
	return (
		<FileTreeFile
			path={PLAZA_VIRTUAL_PATH}
			name={t("plaza.plaza")}
			aria-expanded={expanded}
		>
			<FileTreeDisclosureIcon
				isExpanded={expanded}
				icon={<Globe className="size-4 text-muted-foreground" aria-hidden />}
			/>
			<FileTreeName className="min-w-0 flex-1 truncate">
				{t("plaza.plaza")}
			</FileTreeName>
		</FileTreeFile>
	);
}

export function PlazaSourceRow({ source }: { source: PlazaSource }) {
	const Icon = PLAZA_SOURCE_ICONS[source.icon];
	const label = plazaSourceLabel(source);
	return (
		<FileTreeFile
			path={source.path}
			name={label}
			{...(source.id === "cool-papers" ? { "data-cool-papers": "" } : {})}
		>
			<FileTreeIcon>
				<Icon className="size-4" />
			</FileTreeIcon>
			<FileTreeName className="min-w-0 flex-1 truncate" title={label}>
				{label}
			</FileTreeName>
		</FileTreeFile>
	);
}

/** A reference rendered in another Zotero collection, not a second paper. */
export function ZoteroCollectionReferenceRow({
	path,
	label,
}: {
	path: string;
	label: string;
}) {
	return (
		<FileTreeFile path={path} name={label}>
			<FileTreeIcon>
				<Link2 className="size-4 text-muted-foreground" aria-hidden />
			</FileTreeIcon>
			<FileTreeName className="min-w-0 flex-1 truncate" title={label}>
				<MathText text={label} />
			</FileTreeName>
		</FileTreeFile>
	);
}

export function LoadingRows() {
	return (
		<div className="space-y-1 px-2 py-1.5" aria-hidden>
			{["one", "two", "three", "four", "five"].map((key, index) => (
				<div key={key} className="flex h-7 items-center gap-2 rounded px-2">
					<Skeleton className="size-4 shrink-0 library-shimmer" />
					<Skeleton
						className={cn(
							"library-shimmer h-3",
							index === 0 ? "w-32" : index === 1 ? "w-24" : "w-28",
						)}
					/>
				</div>
			))}
		</div>
	);
}
