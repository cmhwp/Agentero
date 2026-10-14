import { RefreshCw } from "lucide-react";
import type { ReactNode } from "react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import type { ZhihuSearchItem } from "@/lib/core/bindings";
import { errorText } from "@/lib/core/error";
import { openExternalUrl } from "@/lib/core/open-external";
import { cn } from "@/lib/core/utils";
import type { PaperMetadata } from "@/lib/paper";
import { loadSettings } from "@/lib/settings";
import { hasZhihuSecret, searchZhihuDiscussions } from "@/lib/zhihu";

/** Cap the list so the empty state stays compact and never overflows. */
const MAX_CARDS = 5;

type DiscussionState =
	| { kind: "loading" }
	| { kind: "ready"; items: ZhihuSearchItem[] }
	| { kind: "error"; message: string };

function Muted({ children }: { children: ReactNode }) {
	return (
		<p className="max-w-sm text-balance px-2 text-muted-foreground text-sm">
			{children}
		</p>
	);
}

/**
 * Zhihu agent empty state: instead of generic workflow suggestions, show the
 * paper's discussion on Zhihu directly. Runs a title search on mount (and when
 * the focused paper changes) and renders the host-filtered results as cards.
 */
export function ZhihuDiscussions({
	paper,
	className,
}: {
	paper: PaperMetadata | null;
	className?: string;
}) {
	const { t } = useTranslation("agent");
	const title = paper?.title?.trim() ?? "";
	const secretConfigured = hasZhihuSecret(loadSettings().zhihuAccessSecret);
	const [state, setState] = useState<DiscussionState>({ kind: "loading" });
	const [reloadKey, setReloadKey] = useState(0);

	// biome-ignore lint/correctness/useExhaustiveDependencies: reloadKey re-triggers the fetch on demand
	useEffect(() => {
		if (!title || !secretConfigured) return;
		let cancelled = false;
		setState({ kind: "loading" });
		searchZhihuDiscussions(title, 10)
			.then((items) => {
				if (!cancelled) setState({ kind: "ready", items });
			})
			.catch((e) => {
				if (!cancelled) setState({ kind: "error", message: errorText(e) });
			});
		return () => {
			cancelled = true;
		};
	}, [title, secretConfigured, reloadKey]);

	if (!title) {
		return (
			<div className={cn("flex w-full justify-center", className)}>
				<Muted>{t("zhihuDiscussions.noPaper")}</Muted>
			</div>
		);
	}
	if (!secretConfigured) {
		return (
			<div className={cn("flex w-full justify-center", className)}>
				<Muted>{t("zhihuDiscussions.noSecret")}</Muted>
			</div>
		);
	}

	return (
		<div
			className={cn(
				"flex w-full max-w-md flex-col items-stretch gap-2 text-left",
				className,
			)}
		>
			{state.kind === "loading" ? (
				<>
					{[0, 1, 2].map((i) => (
						<Skeleton key={i} className="h-16 w-full rounded-lg" />
					))}
					<p className="mt-1 text-center text-muted-foreground text-xs">
						{t("zhihuDiscussions.loading")}
					</p>
				</>
			) : state.kind === "error" ? (
				<div className="flex flex-col items-center gap-2 py-2">
					<Muted>{state.message}</Muted>
					<Button
						type="button"
						variant="outline"
						size="xs"
						onClick={() => setReloadKey((k) => k + 1)}
					>
						<RefreshCw className="size-3" aria-hidden />
						{t("zhihuDiscussions.retry")}
					</Button>
				</div>
			) : state.items.length === 0 ? (
				<Muted>{t("zhihuDiscussions.empty")}</Muted>
			) : (
				<>
					<p className="text-muted-foreground text-xs">
						{t("zhihuDiscussions.note")}
					</p>
					{state.items.slice(0, MAX_CARDS).map((item) => (
						<ZhihuDiscussionCard key={item.url} item={item} />
					))}
					<Button
						type="button"
						variant="ghost"
						size="xs"
						className="self-center text-muted-foreground"
						onClick={() => setReloadKey((k) => k + 1)}
					>
						<RefreshCw className="size-3" aria-hidden />
						{t("zhihuDiscussions.refresh")}
					</Button>
				</>
			)}
		</div>
	);
}

function ZhihuDiscussionCard({ item }: { item: ZhihuSearchItem }) {
	const { t } = useTranslation("agent");
	const kindLabel =
		item.kind === "answer"
			? t("zhihuDiscussions.answer")
			: t("zhihuDiscussions.article");
	return (
		<button
			type="button"
			onClick={() => openExternalUrl(item.url)}
			className={cn(
				"group flex flex-col gap-1.5 rounded-lg border border-border/70 bg-card px-3 py-2.5 text-left",
				"transition-colors hover:border-border hover:bg-muted/40",
			)}
		>
			<div className="flex items-start justify-between gap-2">
				<span className="line-clamp-2 font-medium text-sm text-foreground">
					{item.title}
				</span>
				<span className="mt-0.5 shrink-0 rounded bg-muted px-1.5 py-0.5 text-caption text-muted-foreground">
					{kindLabel}
				</span>
			</div>
			{item.excerpt.trim() ? (
				<p className="line-clamp-2 text-muted-foreground text-xs leading-relaxed">
					{item.excerpt}
				</p>
			) : null}
			<div className="flex items-center gap-1.5 text-caption text-muted-foreground">
				{item.author.trim() ? (
					<span className="truncate">{item.author}</span>
				) : null}
				{item.author.trim() ? <span aria-hidden>·</span> : null}
				<span className="tabular-nums">
					{t("zhihuDiscussions.votes", {
						votes: item.votes.toLocaleString(),
					})}
				</span>
			</div>
		</button>
	);
}
