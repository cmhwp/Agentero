import { ExternalLink } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { AgentLogo } from "@/components/agent/agent-logo";
import { StatusDot } from "@/components/settings/status-dot";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import { openExternalUrl } from "@/lib/core/open-external";
import {
	hasZhihuSecret,
	probeZhihuSecret,
	ZHIHU_DEVELOPER_PROFILE_URL,
} from "@/lib/zhihu";

type ProbeStatus = "idle" | "probing" | "ok" | "fail";

/**
 * Built-in Zhihu Agent as a common-agent row: brand + connectivity dot on the
 * left, write-only Access Secret editor on the right. The secret lives in durable
 * Host settings (developer.zhihu.com) and is redacted over IPC — the saved value
 * is never shown back to the WebView.
 */
export function ZhihuAgentRow({
	savedSecret,
	onCommit,
}: {
	/** Host-redacted current value (`*`-mask) or "". */
	savedSecret: string;
	onCommit: (value: string) => void;
}) {
	const { t } = useTranslation("settings");
	const [draft, setDraft] = useState("");
	const [status, setStatus] = useState<ProbeStatus>("idle");
	const abortRef = useRef<AbortController | null>(null);

	const runProbe = useCallback(async () => {
		if (!hasZhihuSecret(savedSecret)) {
			setStatus("idle");
			return;
		}
		abortRef.current?.abort();
		const ac = new AbortController();
		abortRef.current = ac;
		setStatus("probing");
		const ok = await probeZhihuSecret(ac.signal);
		if (ac.signal.aborted) return;
		setStatus(ok ? "ok" : "fail");
	}, [savedSecret]);

	useEffect(() => {
		void runProbe();
		return () => {
			abortRef.current?.abort();
		};
	}, [runProbe]);

	const changed = draft.trim().length > 0;
	const commit = () => {
		if (!changed) return;
		onCommit(draft.trim());
		setDraft("");
	};

	const statusLabel = t(
		status === "idle"
			? "agent.zhihu.probeIdle"
			: status === "probing"
				? "agent.zhihu.probeProbing"
				: status === "ok"
					? "agent.zhihu.probeOk"
					: "agent.zhihu.probeFail",
	);

	return (
		<div className="flex min-h-10 items-center justify-between gap-4 border-b border-border/50 px-3.5 py-2.5 last:border-b-0">
			<div className="flex min-w-0 items-center gap-2">
				<AgentLogo template="zhihu" />
				<span className="min-w-0 truncate font-medium text-sm text-foreground">
					{t("agent.zhihu.section")}
				</span>
				<Tooltip>
					<TooltipTrigger asChild>
						<Button
							type="button"
							variant="ghost"
							size="icon-xs"
							className="size-6"
							aria-label={t("agent.zhihu.openKeyPage")}
							onClick={() => void openExternalUrl(ZHIHU_DEVELOPER_PROFILE_URL)}
						>
							<ExternalLink className="size-3.5" />
						</Button>
					</TooltipTrigger>
					<TooltipContent>{t("agent.zhihu.openKeyPage")}</TooltipContent>
				</Tooltip>
				<StatusDot
					tone={
						status === "ok"
							? "ok"
							: status === "probing"
								? "warn"
								: status === "fail"
									? "err"
									: "idle"
					}
					label={statusLabel}
				/>
			</div>
			<div className="flex shrink-0 items-center gap-2">
				<Input
					id="zhihu-access-secret"
					type="password"
					autoComplete="off"
					aria-label={t("agent.zhihu.keyLabel")}
					placeholder={t("agent.zhihu.placeholder")}
					className="h-8 w-56 max-w-[16rem] font-mono text-xs"
					value={draft}
					onChange={(e) => setDraft(e.currentTarget.value)}
					onKeyDown={(e) => {
						if (e.key === "Enter") commit();
					}}
				/>
				<Button
					type="button"
					variant="outline"
					size="xs"
					disabled={!changed}
					onClick={commit}
				>
					{t("agent.zhihu.confirm")}
				</Button>
			</div>
		</div>
	);
}
