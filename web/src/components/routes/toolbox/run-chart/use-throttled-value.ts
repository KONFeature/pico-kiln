import { useEffect, useRef, useState } from "react";

/**
 * Throttles `value` to at most one update per `ms`. The leading change applies
 * immediately (so discrete actions like a zoom button stay instant) while rapid
 * bursts — dragging the minimap — collapse to a single trailing update per
 * interval, keeping expensive downstream work (filter + LTTB) off every frame.
 */
export function useThrottledValue<T>(value: T, ms: number): T {
	const [throttled, setThrottled] = useState(value);
	const lastRef = useRef(0);

	useEffect(() => {
		const elapsed = Date.now() - lastRef.current;
		if (elapsed >= ms) {
			lastRef.current = Date.now();
			setThrottled(value);
			return;
		}
		const id = setTimeout(() => {
			lastRef.current = Date.now();
			setThrottled(value);
		}, ms - elapsed);
		return () => clearTimeout(id);
	}, [value, ms]);

	return throttled;
}
