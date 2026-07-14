// TanStack Query hooks for Pico API endpoints

import {
	type UseQueryOptions,
	useMutation,
	useQuery,
	useQueryClient,
} from "@tanstack/react-query";
import {
	getKilnStatus,
	getMonitoringStatus,
	isTauri,
	refreshKiln,
} from "@/integrations/tauri/kiln-monitor";
import { readFileAsText } from "../utils";
import { type PicoAPIClient, PicoAPIError } from "./client";
import { usePico } from "./context";
import type {
	CancelScheduledResponse,
	DeleteFileResponse,
	FileDirectory,
	KilnConfig,
	KilnStatus,
	ListFilesResponse,
	RunProfileResponse,
	SaveConfigResponse,
	ScheduledStatusResponse,
	ScheduleProfileResponse,
	ShutdownResponse,
	StartTuningResponse,
	StopResponse,
	StopTuningResponse,
	TuningMode,
} from "./types";

// Query keys for TanStack Query
export const picoKeys = {
	status: ["pico", "status"] as const,
	logs: ["pico", "logs"] as const,
	scheduledStatus: ["pico", "scheduled-status"] as const,
	files: (directory: string) => ["files", directory] as const,
	fileContent: (directory: string, filename: string) =>
		["file-content", directory, filename] as const,
	config: ["pico", "config"] as const,
};

/**
 * In a Tauri build a native command triggers an immediate poll so the UI
 * reflects a control action without waiting for the next supervisor tick.
 */
function maybeRefresh() {
	if (isTauri()) void refreshKiln();
}

/**
 * Narrow the optional Pico client to a guaranteed instance, throwing a uniform
 * PicoAPIError when no client is configured yet. Lets every hook drop its own
 * `if (!client)` guard.
 */
function assertClient(
	client: PicoAPIClient | null | undefined,
): asserts client is PicoAPIClient {
	if (!client) {
		throw new PicoAPIError("Pico client not initialized");
	}
}

// === Status Hooks ===

/**
 * Hook to fetch kiln status with smart polling based on current state
 */
export function useKilnStatus(
	options?: Partial<UseQueryOptions<KilnStatus, PicoAPIError>>,
) {
	const { client, isConfigured } = usePico();
	const queryClient = useQueryClient();

	return useQuery<KilnStatus, PicoAPIError>({
		queryKey: picoKeys.status,
		queryFn: async () => {
			assertClient(client);
			// In Tauri the Rust supervisor is the single poller; read its latest
			// snapshot.
			if (isTauri()) {
				const native = await getKilnStatus();
				if (native) return native;
				// No snapshot yet. If the supervisor is configured it is the ONLY
				// poller allowed to touch the 2-connection-limited kiln — never open
				// a second direct fetch. Nudge it and reuse the last cached status
				// (live events also populate the cache) until its first poll lands.
				const health = await getMonitoringStatus();
				if (health.running) {
					void refreshKiln();
					const cached = queryClient.getQueryData<KilnStatus>(picoKeys.status);
					if (cached) return cached;
					throw new PicoAPIError("Waiting for background monitor…");
				}
			}
			return await client.getStatus();
		},
		enabled: isConfigured && Boolean(client),
		// Smart polling based on state
		refetchInterval: (query) => {
			const data = query.state.data;
			if (!data) return 30000; // 30s when no data

			switch (data.state) {
				case "RUNNING":
					return 5000; // 5s - active profile running
				case "TUNING":
					return 2000; // 2s - tuning is more critical
				case "IDLE":
				case "COMPLETE":
					return 30000; // 30s - nothing happening
				case "ERROR":
					return 15000; // 15s - check for recovery
				default:
					return 30000;
			}
		},
		// Always treat status as stale: it's a live safety readout, never "fresh".
		staleTime: 0,
		// Don't refetch on window focus if we're already polling
		refetchOnWindowFocus: false,
		// Retry failed requests with exponential backoff
		retry: 3,
		retryDelay: (attemptIndex) => Math.min(1000 * 2 ** attemptIndex, 30000),
		...options,
	});
}

// === Control Mutations ===

/**
 * Mutation to run a profile
 */
export function useRunProfile() {
	const { client } = usePico();
	const queryClient = useQueryClient();

	return useMutation<RunProfileResponse, PicoAPIError, string>({
		mutationFn: async (profileName: string) => {
			assertClient(client);
			return await client.runProfile(profileName);
		},
		onSuccess: () => {
			// Immediately refetch status after starting a profile
			queryClient.invalidateQueries({ queryKey: picoKeys.status });
			maybeRefresh();
		},
	});
}

/**
 * Mutation to stop the current profile
 */
export function useStopProfile() {
	const { client } = usePico();
	const queryClient = useQueryClient();

	return useMutation<StopResponse, PicoAPIError, void>({
		mutationFn: async () => {
			assertClient(client);
			return await client.stopProfile();
		},
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: picoKeys.status });
			maybeRefresh();
		},
	});
}

/**
 * Mutation to emergency shutdown
 */
export function useShutdown() {
	const { client } = usePico();
	const queryClient = useQueryClient();

	return useMutation<ShutdownResponse, PicoAPIError, void>({
		mutationFn: async () => {
			assertClient(client);
			return await client.shutdown();
		},
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: picoKeys.status });
			maybeRefresh();
		},
	});
}

/**
 * Mutation to clear error state
 */
export function useClearError() {
	const { client } = usePico();
	const queryClient = useQueryClient();

	return useMutation({
		mutationFn: async () => {
			assertClient(client);
			return await client.clearError();
		},
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: picoKeys.status });
			maybeRefresh();
		},
	});
}

/**
 * Mutation to reboot the Pico
 * Note: The Pico will reboot immediately, so the request may timeout
 */
export function useReboot() {
	const { client } = usePico();
	const queryClient = useQueryClient();

	return useMutation<{ message?: string }, PicoAPIError, void>({
		mutationFn: async () => {
			assertClient(client);
			try {
				return await client.reboot();
			} catch (error) {
				// Reboot drops the connection before responding, so a timeout or
				// network failure (no HTTP status) means the command landed. A real
				// HTTP error means it did NOT.
				if (error instanceof PicoAPIError && error.statusCode === undefined) {
					return { message: "Reboot initiated" };
				}
				throw error;
			}
		},
		onSuccess: () => {
			// Invalidate all queries since the Pico is rebooting
			queryClient.invalidateQueries({ queryKey: picoKeys.status });
		},
	});
}

// === Tuning Mutations ===

interface StartTuningParams {
	mode: TuningMode;
	maxTemp?: number;
}

/**
 * Mutation to start PID tuning
 */
export function useStartTuning() {
	const { client } = usePico();
	const queryClient = useQueryClient();

	return useMutation<StartTuningResponse, PicoAPIError, StartTuningParams>({
		mutationFn: async ({ mode, maxTemp }) => {
			assertClient(client);
			return await client.startTuning(mode, maxTemp);
		},
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: picoKeys.status });
			maybeRefresh();
		},
	});
}

/**
 * Mutation to stop PID tuning
 */
export function useStopTuning() {
	const { client } = usePico();
	const queryClient = useQueryClient();

	return useMutation<StopTuningResponse, PicoAPIError, void>({
		mutationFn: async () => {
			assertClient(client);
			return await client.stopTuning();
		},
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: picoKeys.status });
			maybeRefresh();
		},
	});
}

// === Scheduling Mutations ===

interface ScheduleProfileParams {
	profileName: string;
	startTime: number; // Unix timestamp
}

/**
 * Mutation to schedule a profile for delayed start
 */
export function useScheduleProfile() {
	const { client } = usePico();
	const queryClient = useQueryClient();

	return useMutation<
		ScheduleProfileResponse,
		PicoAPIError,
		ScheduleProfileParams
	>({
		mutationFn: async ({ profileName, startTime }) => {
			assertClient(client);
			return await client.scheduleProfile(profileName, startTime);
		},
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: picoKeys.status });
			queryClient.invalidateQueries({ queryKey: picoKeys.scheduledStatus });
		},
	});
}

/**
 * Hook to get scheduled profile status
 */
export function useScheduledStatus(
	options?: Partial<UseQueryOptions<ScheduledStatusResponse, PicoAPIError>>,
) {
	const { client, isConfigured } = usePico();

	return useQuery<ScheduledStatusResponse, PicoAPIError>({
		queryKey: picoKeys.scheduledStatus,
		queryFn: async () => {
			assertClient(client);
			return await client.getScheduledStatus();
		},
		enabled: isConfigured && Boolean(client),
		// Poll every 10 seconds to update countdown
		refetchInterval: 10000,
		refetchOnWindowFocus: false,
		...options,
	});
}

/**
 * Mutation to cancel scheduled profile
 */
export function useCancelScheduled() {
	const { client } = usePico();
	const queryClient = useQueryClient();

	return useMutation<CancelScheduledResponse, PicoAPIError, void>({
		mutationFn: async () => {
			assertClient(client);
			return await client.cancelScheduled();
		},
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: picoKeys.status });
			queryClient.invalidateQueries({ queryKey: picoKeys.scheduledStatus });
		},
	});
}

// === Helper Hooks ===

/**
 * Hook to test connection to Pico
 */
export function useTestConnection() {
	const { client } = usePico();
	const queryClient = useQueryClient();

	return useMutation<boolean, PicoAPIError, void>({
		mutationFn: async () => {
			assertClient(client);
			return await client.testConnection();
		},
		onSuccess: (isConnected) => {
			if (isConnected) {
				// If connection successful, invalidate status to fetch fresh data
				queryClient.invalidateQueries({ queryKey: picoKeys.status });
			}
		},
	});
}

// === File Management Hooks ===

/**
 * Hook to list files in a directory
 * Persisted across sessions and available even when kiln is running
 */
export function useListFiles(
	directory: FileDirectory,
	options?: Partial<UseQueryOptions<ListFilesResponse, PicoAPIError>>,
) {
	const { client, isConfigured } = usePico();
	const { data: status } = useKilnStatus();
	const isFileOpsAvailable = status?.state === "IDLE";

	return useQuery<ListFilesResponse, PicoAPIError>({
		queryKey: picoKeys.files(directory),
		queryFn: async () => {
			assertClient(client);
			return await client.listFiles(directory);
		},
		enabled: isConfigured && Boolean(client),
		// If file operations not available (kiln running), use stale data and don't refetch
		refetchInterval: isFileOpsAvailable ? 30000 : false, // Only refetch every 30s when IDLE
		refetchOnWindowFocus: isFileOpsAvailable, // Only refetch on focus when IDLE
		// Keep data for long time since files don't change often
		staleTime: isFileOpsAvailable ? 1000 * 60 * 5 : Number.POSITIVE_INFINITY, // 5 min when IDLE, never stale when running
		gcTime: 1000 * 60 * 60 * 24, // Keep in cache for 24 hours
		// Show cached data immediately while fetching
		placeholderData: (previousData) => previousData,
		retry: (failureCount, _error) => {
			// If kiln is running, don't retry - just use cached data
			if (!isFileOpsAvailable) return false;
			// Otherwise retry up to 2 times
			return failureCount < 2;
		},
		...options,
	});
}

/**
 * Mutation to delete a file
 * Only works when kiln is IDLE
 */
export function useDeleteFile() {
	const { client } = usePico();
	const queryClient = useQueryClient();

	return useMutation<
		DeleteFileResponse,
		PicoAPIError,
		{ directory: FileDirectory; filename: string }
	>({
		mutationFn: async ({ directory, filename }) => {
			assertClient(client);
			return await client.deleteFile(directory, filename);
		},
		onSuccess: (_data, variables) => {
			queryClient.invalidateQueries({
				queryKey: picoKeys.files(variables.directory),
			});
			queryClient.removeQueries({
				queryKey: picoKeys.fileContent(variables.directory, variables.filename),
			});
		},
	});
}

/**
 * Mutation to delete every file in a directory. The Pico has no bulk-delete
 * endpoint: we issue one DELETE per file, sequentially (it serves one request at
 * a time over WiFi, so a parallel burst just thrashes it). Only works when IDLE.
 */
export function useDeleteAllFiles() {
	const { client } = usePico();
	const queryClient = useQueryClient();

	return useMutation<
		{ deletedCount: number },
		Error,
		{ directory: FileDirectory; filenames: string[] }
	>({
		mutationFn: async ({ directory, filenames }) => {
			assertClient(client);
			const errors: string[] = [];
			let deletedCount = 0;
			for (const filename of filenames) {
				try {
					await client.deleteFile(directory, filename);
					deletedCount++;
				} catch (e) {
					errors.push(
						e instanceof Error ? `${filename}: ${e.message}` : filename,
					);
				}
			}
			if (errors.length > 0) {
				throw new Error(
					`Failed to delete ${errors.length} file(s): ${errors.join("; ")}`,
				);
			}
			return { deletedCount };
		},
		onSettled: (_data, _error, variables) => {
			queryClient.invalidateQueries({
				queryKey: picoKeys.files(variables.directory),
			});
			queryClient.removeQueries({
				queryKey: ["file-content", variables.directory],
			});
		},
	});
}

/**
 * Mutation to upload one or more files. The frontend batches a multi-file
 * selection into sequential single-file PUTs (the Pico streams one upload at a
 * time). JSON is minified client-side to save flash. Only works when IDLE.
 */
export function useUploadFiles() {
	const { client } = usePico();
	const queryClient = useQueryClient();

	return useMutation<
		{ uploadedCount: number },
		Error,
		{ directory: FileDirectory; files: File[] }
	>({
		mutationFn: async ({ directory, files }) => {
			assertClient(client);
			const errors: string[] = [];
			let uploadedCount = 0;
			for (const file of files) {
				try {
					let content = await readFileAsText(file);
					// Minify JSON to save space on the Pico; upload as-is if it doesn't parse.
					if (file.name.endsWith(".json")) {
						try {
							content = JSON.stringify(JSON.parse(content));
						} catch {
							// not valid JSON — leave content untouched
						}
					}
					await client.uploadFile(directory, file.name, content);
					uploadedCount++;
				} catch (e) {
					errors.push(
						e instanceof Error ? `${file.name}: ${e.message}` : file.name,
					);
				}
			}
			if (errors.length > 0) {
				throw new Error(
					`Failed to upload ${errors.length} file(s): ${errors.join("; ")}`,
				);
			}
			return { uploadedCount };
		},
		onSettled: (_data, _error, variables) => {
			queryClient.invalidateQueries({
				queryKey: picoKeys.files(variables.directory),
			});
		},
	});
}

// === Logs Hooks ===

/**
 * Hook to poll the live log tail (GET /api/logs RAM-ring snapshot). Unlike the
 * diag flash files, this works in ANY kiln state (it reads RAM, no IDLE gate),
 * so it can monitor a firing in progress. Pass `paused` to stop polling and
 * `intervalMs` to set the cadence (default 3000ms).
 */
export function useLiveLogs(params?: {
	paused?: boolean;
	intervalMs?: number;
}) {
	const { paused = false, intervalMs = 3000 } = params ?? {};
	const { client, isConfigured } = usePico();

	return useQuery<string, PicoAPIError>({
		queryKey: picoKeys.logs,
		queryFn: async () => {
			assertClient(client);
			return await client.getLogs();
		},
		enabled: isConfigured && Boolean(client),
		refetchInterval: paused ? false : intervalMs,
		staleTime: 0,
		refetchOnWindowFocus: false,
		retry: 2,
	});
}

// === Config Hooks ===

/**
 * Fetch the kiln configuration. Changes rarely, so it is cached for a while and
 * only refetched on demand / after a save invalidation.
 */
export function useKilnConfig(
	options?: Partial<UseQueryOptions<KilnConfig, PicoAPIError>>,
) {
	const { client, isConfigured } = usePico();

	return useQuery<KilnConfig, PicoAPIError>({
		queryKey: picoKeys.config,
		queryFn: async () => {
			assertClient(client);
			return await client.getConfig();
		},
		enabled: isConfigured && Boolean(client),
		staleTime: 1000 * 60 * 5,
		refetchOnWindowFocus: false,
		retry: 2,
		...options,
	});
}

/**
 * Save a sparse config PATCH. On success the config query is invalidated so the
 * page re-seeds from the persisted values.
 */
export function useSaveConfig() {
	const { client } = usePico();
	const queryClient = useQueryClient();

	return useMutation<SaveConfigResponse, PicoAPIError, Record<string, unknown>>(
		{
			mutationFn: async (patch) => {
				assertClient(client);
				return await client.saveConfig(patch);
			},
			onSuccess: () => {
				queryClient.invalidateQueries({ queryKey: picoKeys.config });
			},
		},
	);
}
