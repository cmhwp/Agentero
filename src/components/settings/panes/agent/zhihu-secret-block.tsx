import { ExternalLink, Trash2 } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
	SettingsGroup,
	SettingsRow,
	SettingsSectionLabel,
} from "@/components/settings/settings-layout";
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
 * Built-in Zhihu Agent credential card: one Access Secret from the Zhihu open
 * platform (developer.zhihu.com), stored in durable Host settings and redacted
 * over IPC. The WebView input is write-only — the saved value is never shown.
 */
export function ZhihuSecretBlock({
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

	const statusLabel = t(
		status === "idle"
			? "agent.zhihu.probeIdle"
			: status === "probing"
				? "agent.zhihu.probeProbing"
				: status === "ok"
					? "agent.zhihu.probeOk"
					: "agent.zhihu.probeFail",
	);

	const changed = draft.trim().length > 0;

	return (
		<>
			<SettingsSectionLabel className="mt-4">
				{t("agent.zhihu.section")}
			</SettingsSectionLabel>
			<SettingsGroup>
				<SettingsRow
					label={
						<span className="inline-flex items-center gap-1.5">
							{t("agent.zhihu.keyLabel")}
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
						</span>
					}
					htmlFor="zhihu-access-secret"
				>
					<div className="flex items-center gap-2">
						<Input
							id="zhihu-access-secret"
							type="password"
							autoComplete="off"
							placeholder={t("agent.zhihu.placeholder")}
							className="h-8 w-64 max-w-[18rem] font-mono text-xs"
							value={draft}
							onChange={(e) => setDraft(e.currentTarget.value)}
							onKeyDown={(e) => {
								if (e.key === "Enter" && changed) {
									onCommit(draft.trim());
									setDraft("");
								}
							}}
						/>
						<Button
							type="button"
							variant="outline"
							size="xs"
							disabled={!changed}
							onClick={() => {
								onCommit(draft.trim());
								setDraft("");
							}}
						>
							{t("agent.zhihu.confirm")}
						</Button>
						{hasZhihuSecret(savedSecret) ? (
							<Tooltip>
								<TooltipTrigger asChild>
									<Button
										type="button"
										variant="ghost"
										size="icon"
										aria-label={t("agent.zhihu.clear")}
										onClick={() => onCommit("")}
									>
										<Trash2 className="size-4" />
									</Button>
								</TooltipTrigger>
								<TooltipContent>{t("agent.zhihu.clear")}</TooltipContent>
							</Tooltip>
						) : null}
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									type="button"
									variant="ghost"
									size="icon"
									aria-label={t("agent.zhihu.openKeyPage")}
									onClick={() =>
										void openExternalUrl(ZHIHU_DEVELOPER_PROFILE_URL)
									}
								>
									<ExternalLink className="size-4" />
								</Button>
							</TooltipTrigger>
							<TooltipContent>{t("agent.zhihu.openKeyPage")}</TooltipContent>
						</Tooltip>
					</div>
				</SettingsRow>
			</SettingsGroup>
			<p className="mt-2 mb-3 px-0.5 text-muted-foreground text-xs leading-relaxed">
				{t("agent.zhihu.hint")}
			</p>
		</>
	);
}
