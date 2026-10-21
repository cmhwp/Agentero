import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { commands } from "@/lib/core/bindings";
import { errorText } from "@/lib/core/error";
import { callApiResult } from "@/lib/core/ipc";
import { notifyError } from "@/lib/core/notify";
import { pickVaultDirectory } from "@/lib/vault/pick";

type OwnedLibrary = {
	id: string | null;
	name: string;
	owned: boolean;
};

type ShareSummary = {
	id: string;
	libraryId: string;
	name: string;
	permission: string;
	paperPaths: string[];
};

type OpenedShare = {
	share: ShareSummary;
	files: { key: string; size: number }[];
};

const splitList = (value: string) =>
	value
		.split(/[\n,]/)
		.map((item) => item.trim())
		.filter((item) => item.length > 0);

export function LibrarySharePanel({
	disabled,
	libraries,
}: {
	disabled: boolean;
	libraries: OwnedLibrary[];
}) {
	const { t } = useTranslation("app");
	const owned = libraries.filter((library) => library.owned && library.id);
	const firstOwnedId = owned[0]?.id ?? "";
	const [libraryId, setLibraryId] = useState(firstOwnedId);
	const [name, setName] = useState("");
	const [paths, setPaths] = useState("");
	const [emails, setEmails] = useState("");
	const [permission, setPermission] = useState<"view" | "comment">("view");
	const [createdId, setCreatedId] = useState<string | null>(null);
	const [ownedShares, setOwnedShares] = useState<ShareSummary[]>([]);
	const [received, setReceived] = useState<ShareSummary[]>([]);
	const [opened, setOpened] = useState<OpenedShare | null>(null);
	const [comment, setComment] = useState("");
	const [notice, setNotice] = useState<string | null>(null);
	const [pending, setPending] = useState(false);

	useEffect(() => {
		if (!libraryId && firstOwnedId) setLibraryId(firstOwnedId);
	}, [libraryId, firstOwnedId]);

	const run = async (action: () => Promise<void>) => {
		setPending(true);
		setNotice(null);
		try {
			await action();
		} catch (error) {
			notifyError(errorText(error));
		} finally {
			setPending(false);
		}
	};

	const refreshReceived = async () => {
		const shares = await callApiResult(() => commands.librarySharesReceived());
		setReceived(shares);
	};

	useEffect(() => {
		let cancelled = false;
		void callApiResult(() => commands.librarySharesReceived())
			.then((shares) => {
				if (!cancelled) setReceived(shares);
			})
			.catch(() => {
				if (!cancelled) setReceived([]);
			});
		return () => {
			cancelled = true;
		};
	}, []);

	useEffect(() => {
		if (!libraryId) {
			setOwnedShares([]);
			return;
		}
		let cancelled = false;
		void callApiResult(() => commands.librarySharesOwned({ libraryId }))
			.then((shares) => {
				if (!cancelled) setOwnedShares(shares);
			})
			.catch(() => {
				if (!cancelled) setOwnedShares([]);
			});
		return () => {
			cancelled = true;
		};
	}, [libraryId]);

	const busy = disabled || pending;

	return (
		<div className="overflow-hidden rounded-lg border bg-background shadow-sm">
			<div className="border-b px-3 py-2">
				<p className="font-medium text-muted-foreground text-xs">
					{t("vault.libraries.shareTitle")}
				</p>
			</div>
			<form
				className="flex flex-col gap-2 p-3"
				onSubmit={(event) => {
					event.preventDefault();
					void run(async () => {
						const created = await callApiResult(() =>
							commands.libraryShareCreate({
								libraryId,
								name: name.trim(),
								permission,
								paperPaths: splitList(paths),
								emails: splitList(emails),
							}),
						);
						setCreatedId(created.id);
						setName("");
						setPaths("");
						setEmails("");
						setOwnedShares((current) => [
							created,
							...current.filter((share) => share.id !== created.id),
						]);
					});
				}}
			>
				<label className="flex items-center gap-2 text-muted-foreground text-xs">
					<span className="w-24 shrink-0">{t("vault.libraries.shareLibrary")}</span>
					<select
						className="min-w-0 flex-1 rounded-md border bg-background px-2 py-1 text-foreground"
						disabled={busy || owned.length === 0}
						value={libraryId}
						onChange={(event) => setLibraryId(event.target.value)}
					>
						{owned.map((library) => (
							<option key={library.id} value={library.id ?? ""}>
								{library.name}
							</option>
						))}
					</select>
				</label>
				<Input
					value={name}
					disabled={busy}
					placeholder={t("vault.libraries.shareName")}
					onChange={(event) => setName(event.target.value)}
				/>
				<textarea
					className="min-h-16 rounded-md border bg-background px-2 py-1 text-sm"
					value={paths}
					disabled={busy}
					placeholder={t("vault.libraries.sharePathsPlaceholder")}
					aria-label={t("vault.libraries.sharePaths")}
					onChange={(event) => setPaths(event.target.value)}
				/>
				<textarea
					className="min-h-16 rounded-md border bg-background px-2 py-1 text-sm"
					value={emails}
					disabled={busy}
					placeholder={t("vault.libraries.shareEmailsPlaceholder")}
					aria-label={t("vault.libraries.shareEmails")}
					onChange={(event) => setEmails(event.target.value)}
				/>
				<label className="flex items-center gap-2 text-muted-foreground text-xs">
					<span className="w-24 shrink-0">
						{t("vault.libraries.sharePermission")}
					</span>
					<select
						className="min-w-0 flex-1 rounded-md border bg-background px-2 py-1 text-foreground"
						disabled={busy}
						value={permission}
						onChange={(event) =>
							setPermission(event.target.value === "comment" ? "comment" : "view")
						}
					>
						<option value="view">{t("vault.libraries.shareView")}</option>
						<option value="comment">{t("vault.libraries.shareComment")}</option>
					</select>
				</label>
				<Button
					type="submit"
					size="sm"
					disabled={
						busy ||
						!libraryId ||
						!name.trim() ||
						splitList(paths).length === 0 ||
						splitList(emails).length === 0
					}
				>
					{t("vault.libraries.shareSubmit")}
				</Button>
				{ownedShares.length > 0 && (
					<ul className="flex flex-col gap-1">
						{ownedShares.map((share) => (
							<li
								key={share.id}
								className="flex items-center justify-between gap-2 text-xs"
							>
								<span className="truncate">
									{share.name}: {share.paperPaths.join(", ")}
								</span>
								<Button
									type="button"
									size="sm"
									variant="outline"
									disabled={busy}
									onClick={() =>
										void run(async () => {
											await callApiResult(() =>
												commands.libraryShareRevoke({
													libraryId,
													shareId: share.id,
												}),
											);
											setOwnedShares((current) =>
												current.filter((item) => item.id !== share.id),
											);
											if (createdId === share.id) setCreatedId(null);
										})
									}
								>
									{t("vault.libraries.shareRevoke")}
								</Button>
							</li>
						))}
					</ul>
				)}
			</form>
			<div className="border-t px-3 py-2">
				<div className="mb-2 flex items-center justify-between gap-2">
					<p className="font-medium text-muted-foreground text-xs">
						{t("vault.libraries.shareReceived")}
					</p>
					<Button
						type="button"
						size="sm"
						variant="ghost"
						disabled={busy}
						onClick={() => void run(refreshReceived)}
					>
						{t("vault.libraries.shareRefresh")}
					</Button>
				</div>
				{received.length === 0 ? (
					<p className="text-muted-foreground text-xs">
						{t("vault.libraries.shareEmpty")}
					</p>
				) : (
					<ul className="flex flex-col gap-2">
						{received.map((share) => (
							<li key={share.id} className="rounded-md border px-2 py-2">
								<p className="truncate font-medium text-sm">{share.name}</p>
								<p className="truncate text-muted-foreground text-xs">
									{share.paperPaths.join(", ")}
								</p>
								<div className="mt-2 flex flex-wrap gap-2">
									<Button
										type="button"
										size="sm"
										variant="outline"
										disabled={busy}
										onClick={() =>
											void run(async () => {
												const next = await callApiResult(() =>
													commands.libraryShareOpen({ shareId: share.id }),
												);
												setOpened(next);
											})
										}
									>
										{t("vault.libraries.shareOpen")}
									</Button>
									<Button
										type="button"
										size="sm"
										disabled={busy}
										onClick={() =>
											void run(async () => {
												const path = await pickVaultDirectory();
												if (!path) return;
												await callApiResult(() =>
													commands.libraryShareImport({
														shareId: share.id,
														path,
													}),
												);
												setNotice(t("vault.libraries.shareImported"));
											})
										}
									>
										{t("vault.libraries.shareImport")}
									</Button>
								</div>
							</li>
						))}
					</ul>
				)}
				{opened && (
					<div className="mt-3 flex flex-col gap-2">
						<p className="text-muted-foreground text-xs">
							{t("vault.libraries.shareFiles")}
						</p>
						<ul className="text-xs">
							{opened.files.map((file) => (
								<li key={file.key} className="truncate">
									{file.key} ({file.size})
								</li>
							))}
						</ul>
						{opened.share.permission === "comment" && (
							<form
								className="flex gap-2"
								onSubmit={(event) => {
									event.preventDefault();
									void run(async () => {
										await callApiResult(() =>
											commands.libraryShareComment({
												shareId: opened.share.id,
												body: comment.trim(),
											}),
										);
										setComment("");
									});
								}}
							>
								<Input
									value={comment}
									disabled={busy}
									placeholder={t("vault.libraries.shareCommentBody")}
									onChange={(event) => setComment(event.target.value)}
								/>
								<Button
									type="submit"
									size="sm"
									variant="outline"
									disabled={busy || !comment.trim()}
								>
									{t("vault.libraries.shareCommentSend")}
								</Button>
							</form>
						)}
					</div>
				)}
				{notice && <p className="mt-2 text-muted-foreground text-xs">{notice}</p>}
			</div>
		</div>
	);
}
