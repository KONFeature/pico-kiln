// TanStack Query hooks for Pico API endpoints

import {
	type UseQueryOptions,
	useMutation,
	useQuery,
	useQueryClient,
} from "@tanstack/react-query";
import { PicoAPIError } from "./client";
import { usePico } from "./context";
import type {
	CancelScheduledResponse,
	DeleteAllFilesResponse,
	DeleteFileResponse,
	FileDirectory,
	GetFileResponse,
	KilnStatus,
	ListFilesResponse,
	RunProfileResponse,
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
};

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
			if (!client) {
				throw new PicoAPIError("Pico client not initialized");
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
					return 30000; // 30s - nothing happening
				case "ERROR":
					return 15000; // 15s - check for recovery
				default:
					return 30000;
			}
		},
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
			if (!client) {
				throw new PicoAPIError("Pico client not initialized");
			}
			return await client.runProfile(profileName);
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
			if (!client) {
				throw new PicoAPIError("Pico client not initialized");
			}
			return await client.stopProfile();
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
			if (!client) {
				throw new PicoAPIError("Pico client not initialized");
			}
			return await client.shutdown();
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

	return useMutation<
		{ success: boolean; message: string },
		PicoAPIError,
		void
	>({
		mutationFn: async () => {
			if (!client) {
				throw new PicoAPIError("Pico client not initialized");
			}
			return await client.clearError();
		},
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: picoKeys.status });
		},
	});
}

/**
 * Mutation to reboot the Pico
 */
export function useReboot() {
	const { client } = usePico();
	const queryClient = useQueryClient();

	return useMutation<
		{ success: boolean; message: string },
		PicoAPIError,
		void
	>({
		mutationFn: async () => {
			if (!client) {
				throw new PicoAPIError("Pico client not initialized");
			}
			return await client.reboot();
		},
		onSuccess: () => {
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
			if (!client) {
				throw new PicoAPIError("Pico client not initialized");
			}
			return await client.startTuning(mode, maxTemp);
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
			if (!client) {
				throw new PicoAPIError("Pico client not initialized");
			}
			return await client.stopTuning();
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
			if (!client) {
				throw new PicoAPIError("Pico client not initialized");
			}
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
			if (!client) {
				throw new PicoAPIError("Pico client not initialized");
			}
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
			if (!client) {
				throw new PicoAPIError("Pico client not initialized");
			}
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
			if (!client) {
				throw new PicoAPIError("Pico client not initialized");
			}
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
			if (!client) {
				throw new PicoAPIError("Pico client not initialized");
			}
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
		retry: (failureCount, error) => {
			// If kiln is running, don't retry - just use cached data
			if (!isFileOpsAvailable) return false;
			// Otherwise retry up to 2 times
			return failureCount < 2;
		},
		...options,
	});
}

/**
 * Hook to get file content
 * Persisted across sessions for offline access
 */
export function useGetFile(
	directory: FileDirectory,
	filename: string,
	enabled = true,
	options?: Partial<UseQueryOptions<GetFileResponse, PicoAPIError>>,
) {
	const { client, isConfigured } = usePico();
	const { data: status } = useKilnStatus();
	const isFileOpsAvailable = status?.state === "IDLE";

	return useQuery<GetFileResponse, PicoAPIError>({
		queryKey: picoKeys.fileContent(directory, filename),
		queryFn: async () => {
			if (!client) {
				throw new PicoAPIError("Pico client not initialized");
			}
			return await client.getFile(directory, filename);
		},
		enabled: enabled && isConfigured && Boolean(client),
		// If file operations not available, use cached data
		refetchInterval: false, // Don't auto-refetch file content
		refetchOnWindowFocus: isFileOpsAvailable, // Only refetch on focus when IDLE
		staleTime: isFileOpsAvailable ? 1000 * 60 * 10 : Number.POSITIVE_INFINITY, // 10 min when IDLE, never stale when running
		gcTime: 1000 * 60 * 60 * 24 * 7, // Keep in cache for 7 days
		placeholderData: (previousData) => previousData,
		retry: (failureCount, error) => {
			if (!isFileOpsAvailable) return false;
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
			if (!client) {
				throw new PicoAPIError("Pico client not initialized");
			}
			return await client.deleteFile(directory, filename);
		},
		onSuccess: (data, variables) => {
			if (data.success) {
				// Invalidate file list
				queryClient.invalidateQueries({
					queryKey: picoKeys.files(variables.directory),
				});
				// Remove cached file content
				queryClient.removeQueries({
					queryKey: picoKeys.fileContent(
						variables.directory,
						variables.filename,
					),
				});
			}
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
			if (!client) {
				throw new PicoAPIError("Pico client not initialized");
			}
			return await client.deleteAllLogs();
		},
		onSuccess: (data) => {
			if (data.success) {
				// Invalidate logs file list
				queryClient.invalidateQueries({ queryKey: picoKeys.files("logs") });
				// Remove all cached log file content
				queryClient.removeQueries({ queryKey: ["file-content", "logs"] });
			}
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
			if (!client) {
				throw new PicoAPIError("Pico client not initialized");
			}
			return await client.uploadFile(directory, filename, content);
		},
		onSuccess: (data, variables) => {
			if (data.success) {
				// Invalidate file list to show new file
				queryClient.invalidateQueries({
					queryKey: picoKeys.files(variables.directory),
				});
				// Invalidate cached content for this file in case it was updated
				queryClient.invalidateQueries({
					queryKey: picoKeys.fileContent(
						variables.directory,
						variables.filename,
					),
				});
			}
		},
	});
}
