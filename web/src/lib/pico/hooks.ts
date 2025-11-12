// TanStack Query hooks for Pico API endpoints

import { useMutation, useQuery, useQueryClient, type UseQueryOptions } from '@tanstack/react-query';
import { usePico } from './context';
import type {
  KilnStatus,
  RunProfileResponse,
  StopResponse,
  ShutdownResponse,
  StartTuningResponse,
  StopTuningResponse,
  TuningMode,
  ScheduleProfileResponse,
  ScheduledStatusResponse,
  CancelScheduledResponse,
} from './types';
import { PicoAPIError } from './client';

// Query keys for TanStack Query
export const picoKeys = {
  all: ['pico'] as const,
  status: () => [...picoKeys.all, 'status'] as const,
  tuningStatus: () => [...picoKeys.all, 'tuning-status'] as const,
  scheduledStatus: () => [...picoKeys.all, 'scheduled-status'] as const,
};

// === Status Hooks ===

/**
 * Hook to fetch kiln status with smart polling based on current state
 */
export function useKilnStatus(options?: Partial<UseQueryOptions<KilnStatus, PicoAPIError>>) {
  const { client, isConfigured, updateConnectionHealth } = usePico();

  return useQuery<KilnStatus, PicoAPIError>({
    queryKey: picoKeys.status(),
    queryFn: async () => {
      if (!client) {
        throw new PicoAPIError('Pico client not initialized');
      }
      
      try {
        const status = await client.getStatus();
        updateConnectionHealth(true);
        return status;
      } catch (error) {
        const errorMessage = error instanceof Error ? error.message : 'Unknown error';
        updateConnectionHealth(false, errorMessage);
        throw error;
      }
    },
    enabled: isConfigured && Boolean(client),
    // Smart polling based on state
    refetchInterval: (query) => {
      const data = query.state.data;
      if (!data) return 30000; // 30s when no data

      switch (data.state) {
        case 'RUNNING':
          return 5000; // 5s - active profile running
        case 'TUNING':
          return 2000; // 2s - tuning is more critical
        case 'IDLE':
          return 30000; // 30s - nothing happening
        case 'ERROR':
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

/**
 * Hook to fetch tuning status (alias of useKilnStatus for semantic clarity)
 */
export function useTuningStatus(options?: Partial<UseQueryOptions<KilnStatus, PicoAPIError>>) {
  // Same as useKilnStatus - the API returns the same data
  return useKilnStatus(options);
}

// === Control Mutations ===

/**
 * Mutation to run a profile
 */
export function useRunProfile() {
  const { client, updateConnectionHealth } = usePico();
  const queryClient = useQueryClient();

  return useMutation<RunProfileResponse, PicoAPIError, string>({
    mutationFn: async (profileName: string) => {
      if (!client) {
        throw new PicoAPIError('Pico client not initialized');
      }
      
      try {
        const response = await client.runProfile(profileName);
        updateConnectionHealth(true);
        return response;
      } catch (error) {
        const errorMessage = error instanceof Error ? error.message : 'Unknown error';
        updateConnectionHealth(false, errorMessage);
        throw error;
      }
    },
    onSuccess: () => {
      // Immediately refetch status after starting a profile
      queryClient.invalidateQueries({ queryKey: picoKeys.status() });
    },
  });
}

/**
 * Mutation to stop the current profile
 */
export function useStopProfile() {
  const { client, updateConnectionHealth } = usePico();
  const queryClient = useQueryClient();

  return useMutation<StopResponse, PicoAPIError, void>({
    mutationFn: async () => {
      if (!client) {
        throw new PicoAPIError('Pico client not initialized');
      }
      
      try {
        const response = await client.stopProfile();
        updateConnectionHealth(true);
        return response;
      } catch (error) {
        const errorMessage = error instanceof Error ? error.message : 'Unknown error';
        updateConnectionHealth(false, errorMessage);
        throw error;
      }
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: picoKeys.status() });
    },
  });
}

/**
 * Mutation to emergency shutdown
 */
export function useShutdown() {
  const { client, updateConnectionHealth } = usePico();
  const queryClient = useQueryClient();

  return useMutation<ShutdownResponse, PicoAPIError, void>({
    mutationFn: async () => {
      if (!client) {
        throw new PicoAPIError('Pico client not initialized');
      }
      
      try {
        const response = await client.shutdown();
        updateConnectionHealth(true);
        return response;
      } catch (error) {
        const errorMessage = error instanceof Error ? error.message : 'Unknown error';
        updateConnectionHealth(false, errorMessage);
        throw error;
      }
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: picoKeys.status() });
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
  const { client, updateConnectionHealth } = usePico();
  const queryClient = useQueryClient();

  return useMutation<StartTuningResponse, PicoAPIError, StartTuningParams>({
    mutationFn: async ({ mode, maxTemp }) => {
      if (!client) {
        throw new PicoAPIError('Pico client not initialized');
      }
      
      try {
        const response = await client.startTuning(mode, maxTemp);
        updateConnectionHealth(true);
        return response;
      } catch (error) {
        const errorMessage = error instanceof Error ? error.message : 'Unknown error';
        updateConnectionHealth(false, errorMessage);
        throw error;
      }
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: picoKeys.status() });
    },
  });
}

/**
 * Mutation to stop PID tuning
 */
export function useStopTuning() {
  const { client, updateConnectionHealth } = usePico();
  const queryClient = useQueryClient();

  return useMutation<StopTuningResponse, PicoAPIError, void>({
    mutationFn: async () => {
      if (!client) {
        throw new PicoAPIError('Pico client not initialized');
      }
      
      try {
        const response = await client.stopTuning();
        updateConnectionHealth(true);
        return response;
      } catch (error) {
        const errorMessage = error instanceof Error ? error.message : 'Unknown error';
        updateConnectionHealth(false, errorMessage);
        throw error;
      }
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: picoKeys.status() });
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
  const { client, updateConnectionHealth } = usePico();
  const queryClient = useQueryClient();

  return useMutation<ScheduleProfileResponse, PicoAPIError, ScheduleProfileParams>({
    mutationFn: async ({ profileName, startTime }) => {
      if (!client) {
        throw new PicoAPIError('Pico client not initialized');
      }
      
      try {
        const response = await client.scheduleProfile(profileName, startTime);
        updateConnectionHealth(true);
        return response;
      } catch (error) {
        const errorMessage = error instanceof Error ? error.message : 'Unknown error';
        updateConnectionHealth(false, errorMessage);
        throw error;
      }
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: picoKeys.status() });
      queryClient.invalidateQueries({ queryKey: picoKeys.scheduledStatus() });
    },
  });
}

/**
 * Hook to get scheduled profile status
 */
export function useScheduledStatus(options?: Partial<UseQueryOptions<ScheduledStatusResponse, PicoAPIError>>) {
  const { client, isConfigured, updateConnectionHealth } = usePico();

  return useQuery<ScheduledStatusResponse, PicoAPIError>({
    queryKey: picoKeys.scheduledStatus(),
    queryFn: async () => {
      if (!client) {
        throw new PicoAPIError('Pico client not initialized');
      }
      
      try {
        const status = await client.getScheduledStatus();
        updateConnectionHealth(true);
        return status;
      } catch (error) {
        const errorMessage = error instanceof Error ? error.message : 'Unknown error';
        updateConnectionHealth(false, errorMessage);
        throw error;
      }
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
  const { client, updateConnectionHealth } = usePico();
  const queryClient = useQueryClient();

  return useMutation<CancelScheduledResponse, PicoAPIError, void>({
    mutationFn: async () => {
      if (!client) {
        throw new PicoAPIError('Pico client not initialized');
      }
      
      try {
        const response = await client.cancelScheduled();
        updateConnectionHealth(true);
        return response;
      } catch (error) {
        const errorMessage = error instanceof Error ? error.message : 'Unknown error';
        updateConnectionHealth(false, errorMessage);
        throw error;
      }
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: picoKeys.status() });
      queryClient.invalidateQueries({ queryKey: picoKeys.scheduledStatus() });
    },
  });
}

// === Helper Hooks ===

/**
 * Hook to test connection to Pico
 */
export function useTestConnection() {
  const { testConnection } = usePico();
  const queryClient = useQueryClient();

  return useMutation<boolean, Error, void>({
    mutationFn: async () => {
      return await testConnection();
    },
    onSuccess: (isConnected) => {
      if (isConnected) {
        // If connection successful, invalidate status to fetch fresh data
        queryClient.invalidateQueries({ queryKey: picoKeys.status() });
      }
    },
  });
}
