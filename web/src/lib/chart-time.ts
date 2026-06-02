/**
 * bklit charts render on a time scale, so kiln "elapsed time" series are mapped
 * onto Date objects offset from the Unix epoch (elapsed 0s → 1970-01-01T00:00Z).
 * Reading `.getTime()` back yields the elapsed milliseconds independent of the
 * viewer's timezone (we never use local-time getters for elapsed values).
 */
export function elapsedSecondsToDate(seconds: number): Date {
	return new Date(seconds * 1000);
}

/** Elapsed-time label for axis ticks / tooltips: "45s", "12m", "1.5h", "13h". */
export function formatElapsed(totalSeconds: number): string {
	if (!Number.isFinite(totalSeconds)) {
		return "";
	}
	const abs = Math.abs(totalSeconds);
	if (abs >= 3600) {
		const hours = totalSeconds / 3600;
		if (Math.abs(hours) >= 10) {
			return `${Math.round(hours)}h`;
		}
		const oneDecimal = Math.round(hours * 10) / 10;
		return Number.isInteger(oneDecimal)
			? `${oneDecimal}h`
			: `${oneDecimal.toFixed(1)}h`;
	}
	if (abs >= 60) {
		return `${Math.round(totalSeconds / 60)}m`;
	}
	return `${Math.round(totalSeconds)}s`;
}
