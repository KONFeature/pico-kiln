// TanStack Query hooks for Pico API endpoints

import {
	type UseQueryOptions,
	useMutation,
	useQuery,
	useQueryClient,
} from "@tanstack/react-query";
import { type PicoAPIClient, PicoAPIError } from "./client";
import { usePico } from "./context";
import { isLogicalFailure } from "./errors";
import type {
	CancelScheduledResponse,
	DeleteAllFilesResponse,
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
	UploadFileResponse,
} from "./types";

// Query keys for TanStack Query
export const picoKeys = {
	status: ["pico", "status"] as const,
	scheduledStatus: ["pico", "scheduled-status"] as const,
	files: (directory: string) => ["files", directory] as const,
	fileContent: (directory: string, filename: string) =>
		["file-content", directory, filename] as const,
	config: ["pico", "config"] as const,
};

/**
 * Rethrow logical failures (`{ success: false }` returned with HTTP 200) as a
 * PicoAPIError so mutations surface them through `isError` instead of resolving
 * silently. The original response is attached so `getFriendlyError` can detect
 * the device-provided reason and trust it.
 */
function unwrap<
	T extends { success: boolean; error?: string; message?: string },
>(res: T, fallback: string): T {
	if (!res.success) {
		throw new PicoAPIError(
			res.error ?? res.message ?? fallback,
			undefined,
			res,
		);
	}
	return res;
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

	return useQuery<KilnStatus, PicoAPIError>({
		queryKey: picoKeys.status,
		queryFn: async () => {
			assertClient(client);
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
			return unwrap(
				await client.runProfile(profileName),
				"Failed to start profile",
			);
		},
		onSuccess: () => {
			// Immediately refetch status after starting a profile
			queryClient.invalidateQueries({ queryKey: picoKeys.status });
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
			return unwrap(await client.stopProfile(), "Failed to stop profile");
		},
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: picoKeys.status });
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
			return unwrap(await client.shutdown(), "Failed to shut down kiln");
		},
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: picoKeys.status });
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
			return unwrap(await client.clearError(), "Failed to clear error");
		},
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: picoKeys.status });
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

	return useMutation<{ success: boolean; message: string }, PicoAPIError, void>(
		{
			mutationFn: async () => {
				assertClient(client);
				try {
					return unwrap(await client.reboot(), "Failed to reboot");
				} catch (error) {
					// Reboot drops the connection before responding, so a timeout or
					// network failure (no HTTP status, and not an explicit
					// `{ success: false }`) means the command landed. A real HTTP error
					// or a logical rejection means it did NOT.
					if (
						error instanceof PicoAPIError &&
						error.statusCode === undefined &&
						!isLogicalFailure(error)
					) {
						return { success: true, message: "Reboot initiated" };
					}
					throw error;
				}
			},
			onSuccess: () => {
				// Invalidate all queries since the Pico is rebooting
				queryClient.invalidateQueries({ queryKey: picoKeys.status });
			},
		},
	);
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
			return unwrap(
				await client.startTuning(mode, maxTemp),
				"Failed to start tuning",
			);
		},
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: picoKeys.status });
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
			return unwrap(await client.stopTuning(), "Failed to stop tuning");
		},
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: picoKeys.status });
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
			return unwrap(
				await client.scheduleProfile(profileName, startTime),
				"Failed to schedule profile",
			);
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
			return unwrap(
				await client.cancelScheduled(),
				"Failed to cancel scheduled profile",
			);
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
			return unwrap(
				await client.deleteFile(directory, filename),
				"Failed to delete file",
			);
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
 * Mutation to delete all log files
 * Only works when kiln is IDLE
 */
export function useDeleteAllLogs() {
	const { client } = usePico();
	const queryClient = useQueryClient();

	return useMutation<DeleteAllFilesResponse, PicoAPIError, void>({
		mutationFn: async () => {
			assertClient(client);
			return unwrap(await client.deleteAllLogs(), "Failed to delete logs");
		},
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: picoKeys.files("logs") });
			queryClient.removeQueries({ queryKey: ["file-content", "logs"] });
		},
	});
}

/**
 * Mutation to upload a file
 * Only works when kiln is IDLE
 */
export function useUploadFile() {
	const { client } = usePico();
	const queryClient = useQueryClient();

	return useMutation<
		UploadFileResponse,
		PicoAPIError,
		{ directory: FileDirectory; filename: string; content: string }
	>({
		mutationFn: async ({ directory, filename, content }) => {
			assertClient(client);
			return unwrap(
				await client.uploadFile(directory, filename, content),
				"Failed to upload file",
			);
		},
		onSuccess: (_data, variables) => {
			queryClient.invalidateQueries({
				queryKey: picoKeys.files(variables.directory),
			});
			queryClient.invalidateQueries({
				queryKey: picoKeys.fileContent(variables.directory, variables.filename),
			});
		},
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
				return unwrap(await client.saveConfig(patch), "Failed to save config");
			},
			onSuccess: () => {
				queryClient.invalidateQueries({ queryKey: picoKeys.config });
			},
		},
	);
}
