import { Cloud, FolderOpen, LogIn, LogOut } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { commands } from "@/lib/core/bindings";
import { callApiResult } from "@/lib/core/ipc";
import { displayPath } from "@/lib/core/path";
import { notifyError } from "@/lib/core/notify";
import { errorText } from "@/lib/core/error";
import { pickVaultDirectory } from "@/lib/vault/pick";

type LibraryUser = {
	id: string;
	name: string;
	email: string;
};

type LibraryListItem = {
	id: string | null;
	name: string;
	path: string | null;
	connected: boolean;
	onServer: boolean;
};

type LibraryAccountSnapshot = {
	signedIn: boolean;
	user: LibraryUser | null;
	personalCloudBytes: number;
	planKey: string | null;
	planVersion: number | null;
	remoteError: string | null;
	libraries: LibraryListItem[];
};

export function LibraryPanel({
	recentVaults,
	busy,
	onOpenPath,
	onSignedInChange,
}: {
	recentVaults: string[];
	busy?: boolean;
	onOpenPath: (path: string) => void;
	onSignedInChange: (signedIn: boolean) => void;
}) {
	const { t } = useTranslation("app");
	const [account, setAccount] = useState<LibraryAccountSnapshot | null>(null);
	const [pending, setPending] = useState(false);
	const [name, setName] = useState("");
	const [showPaste, setShowPaste] = useState(false);
	const [token, setToken] = useState("");

	const refresh = async () => {
		const snapshot = await callApiResult(() =>
			commands.libraryList({ recentPaths: recentVaults }),
		);
		setAccount(snapshot);
		onSignedInChange(snapshot.signedIn);
		return snapshot;
	};

	useEffect(() => {
		let cancelled = false;
		void callApiResult(() =>
			commands.libraryList({ recentPaths: recentVaults }),
		)
			.then((snapshot) => {
				if (cancelled) return;
				setAccount(snapshot);
				onSignedInChange(snapshot.signedIn);
			})
			.catch((error) => {
				if (!cancelled) notifyError(errorText(error));
			});
		return () => {
			cancelled = true;
		};
	}, [recentVaults, onSignedInChange]);

	const run = async (action: () => Promise<LibraryAccountSnapshot>) => {
		setPending(true);
		try {
			const snapshot = await action();
			setAccount(snapshot);
			onSignedInChange(snapshot.signedIn);
		} catch (error) {
			notifyError(errorText(error));
		} finally {
			setPending(false);
		}
	};

	const disabled = busy || pending;
	const signedIn = account?.signedIn ?? false;

	return (
		<div className="flex flex-col gap-3">
			<div className="flex flex-wrap items-center justify-between gap-2">
				{signedIn ? (
					<p className="min-w-0 truncate text-muted-foreground text-xs">
						{t("vault.libraries.signedInAs", {
							name: account?.user?.name || account?.user?.email || "",
						})}
					</p>
				) : (
					<p className="text-muted-foreground text-xs">
						{t("vault.libraries.title")}
					</p>
				)}
				{signedIn ? (
					<Button
						type="button"
						variant="ghost"
						size="sm"
						disabled={disabled}
						onClick={() =>
							void run(() =>
								callApiResult(() =>
									commands.librarySignOut({ recentPaths: recentVaults }),
								),
							)
						}
					>
						<LogOut className="size-3.5" />
						{t("vault.libraries.signOut")}
					</Button>
				) : (
					<Button
						type="button"
						variant="outline"
						size="sm"
						disabled={disabled}
						onClick={() =>
							void run(() =>
								callApiResult(() =>
									commands.librarySignIn({ recentPaths: recentVaults }),
								),
							)
						}
					>
						<LogIn className="size-3.5" />
						{pending
							? t("vault.libraries.signingIn")
							: t("vault.libraries.signIn")}
					</Button>
				)}
			</div>

			{signedIn && (
				<p className="flex items-center gap-1.5 text-muted-foreground text-xs">
					<Cloud className="size-3.5" />
					{t("vault.libraries.cloudQuota", {
						bytes: account?.personalCloudBytes ?? 0,
					})}
				</p>
			)}
			{account?.remoteError && (
				<p className="text-destructive text-xs">{account.remoteError}</p>
			)}

			{signedIn && (
				<div className="overflow-hidden rounded-lg border bg-background shadow-sm">
					<div className="border-b px-3 py-2">
						<p className="font-medium text-muted-foreground text-xs">
							{t("vault.libraries.title")}
						</p>
					</div>
					{account?.libraries.length ? (
						<ul className="max-h-56 divide-y overflow-y-auto">
							{account.libraries.map((library) => {
								const key = library.id ?? library.path ?? library.name;
								return (
									<li
										key={key}
										className="flex items-center gap-2 px-3 py-2.5"
									>
										<div className="min-w-0 flex-1">
											<p className="truncate font-medium text-sm">
												{library.name}
											</p>
											<p
												className="truncate text-caption text-muted-foreground"
												title={library.path ?? undefined}
											>
												{library.path
													? displayPath(library.path)
													: t("vault.libraries.unconnected")}
											</p>
										</div>
										{library.connected && library.path ? (
											<Button
												type="button"
												variant="ghost"
												size="sm"
												disabled={disabled}
												onClick={() => onOpenPath(library.path as string)}
											>
												<FolderOpen className="size-3.5" />
												{t("vault.openVaultButton")}
											</Button>
										) : (
											<Button
												type="button"
												variant="outline"
												size="sm"
												disabled={disabled || !library.id}
												onClick={() =>
													void run(async () => {
														const path = await pickVaultDirectory();
														if (!path || !library.id) {
															return refresh();
														}
														return callApiResult(() =>
															commands.libraryConnect({
																id: library.id,
																path,
																name: library.name,
																recentPaths: recentVaults,
															}),
														);
													})
												}
											>
												{t("vault.libraries.chooseFolder")}
											</Button>
										)}
									</li>
								);
							})}
						</ul>
					) : (
						<p className="px-3 py-3 text-muted-foreground text-xs">
							{t("vault.libraries.empty")}
						</p>
					)}
					<form
						className="flex gap-2 border-t p-2"
						onSubmit={(event) => {
							event.preventDefault();
							void run(async () => {
								const path = await pickVaultDirectory();
								if (!path) return refresh();
								const snapshot = await callApiResult(() =>
									commands.libraryRegister({
										path,
										displayName: name.trim(),
										recentPaths: recentVaults,
									}),
								);
								setName("");
								onOpenPath(path);
								return snapshot;
							});
						}}
					>
						<Input
							value={name}
							disabled={disabled}
							placeholder={t("vault.libraries.namePlaceholder")}
							onChange={(event) => setName(event.target.value)}
						/>
						<Button type="submit" size="sm" disabled={disabled || !name.trim()}>
							{t("vault.libraries.add")}
						</Button>
					</form>
				</div>
			)}

			{!signedIn && (
				<div className="flex flex-col gap-2">
					<button
						type="button"
						className="self-start text-muted-foreground text-xs underline-offset-2 hover:underline"
						onClick={() => setShowPaste((open) => !open)}
					>
						{t("vault.libraries.pasteToggle")}
					</button>
					{showPaste && (
						<form
							className="flex gap-2"
							onSubmit={(event) => {
								event.preventDefault();
								void run(async () => {
									const snapshot = await callApiResult(() =>
										commands.librarySessionSet({
											token: token.trim(),
											recentPaths: recentVaults,
										}),
									);
									setToken("");
									setShowPaste(false);
									return snapshot;
								});
							}}
						>
							<Input
								value={token}
								disabled={disabled}
								placeholder={t("vault.libraries.pastePlaceholder")}
								onChange={(event) => setToken(event.target.value)}
								autoComplete="off"
								spellCheck={false}
							/>
							<Button
								type="submit"
								size="sm"
								variant="outline"
								disabled={disabled || !token.trim()}
							>
								{t("vault.libraries.pasteSubmit")}
							</Button>
						</form>
					)}
				</div>
			)}
		</div>
	);
}
