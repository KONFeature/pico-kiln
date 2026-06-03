export type SeriesKey = "temp" | "target" | "ssr" | "rate" | "error";

export const MIN_SPAN_S = 30;

export interface SeriesMeta {
	key: SeriesKey;
	label: string;
	color: string;
}

export const SERIES_META: Record<SeriesKey, SeriesMeta> = {
	temp: { key: "temp", label: "Current", color: "var(--chart-heating)" },
	target: { key: "target", label: "Target", color: "var(--muted-foreground)" },
	ssr: { key: "ssr", label: "Heat (SSR)", color: "var(--chart-ssr)" },
	rate: { key: "rate", label: "Rate", color: "var(--chart-rate)" },
	error: { key: "error", label: "Error band", color: "var(--destructive)" },
};
