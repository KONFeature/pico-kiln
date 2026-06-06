import type * as React from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import {
	LOG_LEVELS,
	type LogEntry,
	type LogLevel,
	uniqueTags,
} from "@/lib/diag-parser";
import { cn } from "@/lib/utils";

interface LogViewProps {
	entries: LogEntry[];
	autoScroll?: boolean; // when true, stick to bottom on new entries unless the user scrolled up
	className?: string;
	emptyMessage?: string; // shown when entries is empty (default e.g. "No log lines.")
}

/** Distance from the bottom (px) within which auto-scroll stays engaged. */
const BOTTOM_THRESHOLD = 24;

/** Text color class for a line, keyed by its level (raw lines are dimmed). */
function levelTextClass(level?: LogLevel): string {
	switch (level) {
		case "ERROR":
			return "text-destructive";
		case "WARN":
			return "text-amber-500";
		case "INFO":
			return "text-foreground";
		case "DEBUG":
		case "TRACE":
			return "text-muted-foreground";
		default:
			return "text-muted-foreground";
	}
}

/** Badge variant for a level chip / inline label. */
function levelBadgeClass(level: LogLevel): string {
	switch (level) {
		case "ERROR":
			return "bg-destructive/15 text-destructive border-destructive/30";
		case "WARN":
			return "bg-amber-500/15 text-amber-600 border-amber-500/30 dark:text-amber-400";
		case "INFO":
			return "bg-primary/15 text-primary border-primary/30";
		default:
			return "bg-muted text-muted-foreground border-border";
	}
}

function ToggleChip({
	active,
	onClick,
	className,
	children,
}: {
	active: boolean;
	onClick: () => void;
	className?: string;
	children: React.ReactNode;
}) {
	return (
		<button
			type="button"
			data-active={active}
			onClick={onClick}
			className={cn(
				"rounded-full border px-2 py-0.5 text-xs font-medium transition-colors",
				active
					? className
					: "border-border bg-transparent text-muted-foreground opacity-50 hover:opacity-80",
			)}
		>
			{children}
		</button>
	);
}

export function LogView({
	entries,
	autoScroll,
	className,
	emptyMessage = "No log lines.",
}: LogViewProps): React.JSX.Element {
	const tags = useMemo(() => uniqueTags(entries), [entries]);

	const [enabledLevels, setEnabledLevels] = useState<Set<LogLevel>>(
		() => new Set(LOG_LEVELS),
	);
	const [disabledTags, setDisabledTags] = useState<Set<string>>(
		() => new Set(),
	);
	const [search, setSearch] = useState("");

	const scrollRef = useRef<HTMLDivElement>(null);
	const stickToBottom = useRef(true);

	const filtered = useMemo(() => {
		const needle = search.trim().toLowerCase();
		return entries.filter((entry) => {
			// Entries without a level are always shown; otherwise honor the toggles.
			if (entry.level !== undefined && !enabledLevels.has(entry.level)) {
				return false;
			}
			// Entries without a tag are always shown; otherwise honor the toggles.
			if (entry.tag !== undefined && disabledTags.has(entry.tag)) {
				return false;
			}
			if (needle && !entry.raw.toLowerCase().includes(needle)) {
				return false;
			}
			return true;
		});
	}, [entries, enabledLevels, disabledTags, search]);

	function toggleLevel(level: LogLevel) {
		setEnabledLevels((prev) => {
			const next = new Set(prev);
			if (next.has(level)) {
				next.delete(level);
			} else {
				next.add(level);
			}
			return next;
		});
	}

	function toggleTag(tag: string) {
		setDisabledTags((prev) => {
			const next = new Set(prev);
			if (next.has(tag)) {
				next.delete(tag);
			} else {
				next.add(tag);
			}
			return next;
		});
	}

	function handleScroll() {
		const el = scrollRef.current;
		if (!el) return;
		const distance = el.scrollHeight - el.scrollTop - el.clientHeight;
		stickToBottom.current = distance <= BOTTOM_THRESHOLD;
	}

	// Auto-scroll to the bottom on new entries, but only while the user is
	// already pinned to the bottom (so reading scrollback isn't interrupted).
	// biome-ignore lint/correctness/useExhaustiveDependencies: filtered.length is a re-run trigger (new lines rendered), not read in the body.
	useEffect(() => {
		if (!autoScroll) return;
		const el = scrollRef.current;
		if (!el || !stickToBottom.current) return;
		el.scrollTop = el.scrollHeight;
	}, [autoScroll, filtered.length]);

	if (entries.length === 0) {
		return (
			<div
				className={cn(
					"flex min-h-24 items-center justify-center p-6 text-sm text-muted-foreground",
					className,
				)}
			>
				{emptyMessage}
			</div>
		);
	}

	return (
		<div className={cn("flex flex-col gap-2", className)}>
			<div className="flex flex-wrap items-center gap-2">
				<div className="flex flex-wrap gap-1">
					{LOG_LEVELS.map((level) => (
						<ToggleChip
							key={level}
							active={enabledLevels.has(level)}
							onClick={() => toggleLevel(level)}
							className={levelBadgeClass(level)}
						>
							{level}
						</ToggleChip>
					))}
				</div>
				{tags.length > 0 && (
					<div className="flex flex-wrap gap-1">
						{tags.map((tag) => (
							<ToggleChip
								key={tag}
								active={!disabledTags.has(tag)}
								onClick={() => toggleTag(tag)}
								className="border-border bg-muted text-foreground"
							>
								{tag}
							</ToggleChip>
						))}
					</div>
				)}
				<Input
					type="search"
					value={search}
					onChange={(e) => setSearch(e.target.value)}
					placeholder="Search…"
					className="h-8 w-full sm:ml-auto sm:w-48"
				/>
			</div>

			<div className="text-xs text-muted-foreground">
				{filtered.length} / {entries.length} lines
			</div>

			<div
				ref={scrollRef}
				onScroll={handleScroll}
				className="max-h-[60vh] overflow-y-auto rounded-md border bg-muted/30 p-2 font-mono text-xs leading-relaxed"
			>
				{filtered.length === 0 ? (
					<div className="p-2 text-muted-foreground">
						No lines match the current filters.
					</div>
				) : (
					filtered.map((entry, i) => (
						<div
							key={i}
							className={cn(
								"flex gap-2 whitespace-pre-wrap break-all",
								levelTextClass(entry.level),
							)}
						>
							{entry.level === undefined ? (
								<span>{entry.raw}</span>
							) : (
								<>
									<span className="shrink-0 text-muted-foreground">
										{entry.time}
									</span>
									<Badge
										variant="outline"
										className={cn(
											"shrink-0 px-1 py-0 font-mono",
											levelBadgeClass(entry.level),
										)}
									>
										{entry.level}
									</Badge>
									{entry.tag !== undefined && (
										<Badge
											variant="outline"
											className="shrink-0 px-1 py-0 font-mono text-muted-foreground"
										>
											{entry.tag}
										</Badge>
									)}
									<span className="min-w-0">{entry.message}</span>
								</>
							)}
						</div>
					))
				)}
			</div>
		</div>
	);
}
