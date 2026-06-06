// @vitest-environment jsdom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { ReactElement } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

// Hoisted so the vi.mock factories below (which are themselves hoisted) can
// reference them without hitting the temporal dead zone.
const { setPicoURL, testConnection, clientCtor } = vi.hoisted(() => ({
	setPicoURL: vi.fn(),
	testConnection: vi.fn(),
	clientCtor: vi.fn(),
}));

vi.mock("@/lib/pico/context", () => ({
	usePico: () => ({ setPicoURL }),
}));

vi.mock("@/lib/pico/client", () => ({
	PicoAPIClient: class {
		testConnection = testConnection;
		constructor(url: string, timeoutMs?: number) {
			clientCtor(url, timeoutMs);
		}
	},
	PicoAPIError: class extends Error {},
}));

import { PicoQuickConnect } from "./PicoQuickConnect";

// getBy*/findBy* throw when nothing matches, so reaching the assertion already
// proves presence; the truthy check keeps biome's no-unused happy without
// @testing-library/jest-dom (not a dependency here).
function renderUI(ui: ReactElement) {
	const queryClient = new QueryClient();
	return render(
		<QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>,
	);
}

afterEach(() => {
	// Auto-cleanup isn't registered without vitest `globals`, so unmount manually
	// to stop renders from accumulating in document.body across tests.
	cleanup();
	vi.clearAllMocks();
});

describe("PicoQuickConnect", () => {
	it("renders the USB and Access Point presets with their fixed IPs", () => {
		renderUI(<PicoQuickConnect />);

		expect(screen.getByRole("button", { name: /USB Cable/ })).toBeTruthy();
		expect(screen.getByRole("button", { name: /Access Point/ })).toBeTruthy();
		expect(screen.getByText("192.168.7.1")).toBeTruthy();
		expect(screen.getByText("192.168.4.1")).toBeTruthy();
	});

	it("on a successful USB probe: saves the URL, mirrors it, probes a fresh client, and shows connected", async () => {
		testConnection.mockResolvedValue(true);
		const onPick = vi.fn();
		renderUI(<PicoQuickConnect onPick={onPick} />);

		fireEvent.click(screen.getByRole("button", { name: /USB Cable/ }));

		// Verdict glyph proves the async path completed.
		expect(await screen.findByLabelText("Connected")).toBeTruthy();
		expect(setPicoURL).toHaveBeenCalledWith("http://192.168.7.1");
		expect(onPick).toHaveBeenCalledWith("http://192.168.7.1");
		// Fresh client bound to the chosen URL (the stale-closure fix), short timeout.
		expect(clientCtor).toHaveBeenCalledWith("http://192.168.7.1", 6000);
		expect(testConnection).toHaveBeenCalledTimes(1);
	});

	it("uses the SoftAP address for the Access Point preset", async () => {
		testConnection.mockResolvedValue(true);
		renderUI(<PicoQuickConnect />);

		fireEvent.click(screen.getByRole("button", { name: /Access Point/ }));

		expect(await screen.findByLabelText("Connected")).toBeTruthy();
		expect(setPicoURL).toHaveBeenCalledWith("http://192.168.4.1");
		expect(clientCtor).toHaveBeenCalledWith("http://192.168.4.1", 6000);
	});

	it("on a failed probe: shows the failure glyph and a guidance message", async () => {
		testConnection.mockResolvedValue(false);
		renderUI(<PicoQuickConnect />);

		fireEvent.click(screen.getByRole("button", { name: /USB Cable/ }));

		expect(await screen.findByLabelText("Failed")).toBeTruthy();
		expect(screen.getByText(/Couldn't reach the kiln/)).toBeTruthy();
	});
});
