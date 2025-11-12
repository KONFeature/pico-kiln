// API client for communicating with the Pico Kiln controller

import type {
  KilnStatus,
  RunProfileRequest,
  RunProfileResponse,
  StopResponse,
  ShutdownResponse,
  StartTuningRequest,
  StartTuningResponse,
  StopTuningResponse,
  ScheduleProfileRequest,
  ScheduleProfileResponse,
  ScheduledStatusResponse,
  CancelScheduledResponse,
} from './types';

export class PicoAPIError extends Error {
  constructor(
    message: string,
    public statusCode?: number,
    public originalError?: unknown
  ) {
    super(message);
    this.name = 'PicoAPIError';
  }
}

export class PicoAPIClient {
  private baseURL: string;
  private timeoutMs: number;

  constructor(baseURL: string, timeoutMs = 10000) {
    // Ensure baseURL doesn't end with a slash
    this.baseURL = baseURL.replace(/\/$/, '');
    this.timeoutMs = timeoutMs;
  }

  private async fetchWithTimeout(
    url: string,
    options: RequestInit = {}
  ): Promise<Response> {
    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), this.timeoutMs);

    try {
      const response = await fetch(url, {
        ...options,
        signal: controller.signal,
      });
      clearTimeout(timeoutId);
      return response;
    } catch (error) {
      clearTimeout(timeoutId);
      if (error instanceof Error && error.name === 'AbortError') {
        throw new PicoAPIError(
          `Request timeout after ${this.timeoutMs}ms`,
          undefined,
          error
        );
      }
      throw new PicoAPIError(
        'Network request failed',
        undefined,
        error
      );
    }
  }

  private async request<T>(
    endpoint: string,
    options: RequestInit = {}
  ): Promise<T> {
    const url = `${this.baseURL}${endpoint}`;

    try {
      const response = await this.fetchWithTimeout(url, options);

      if (!response.ok) {
        const errorText = await response.text().catch(() => 'Unknown error');
        throw new PicoAPIError(
          `HTTP ${response.status}: ${errorText}`,
          response.status
        );
      }

      const contentType = response.headers.get('content-type');
      if (contentType?.includes('application/json')) {
        return await response.json();
      }

      // If not JSON, return empty object as fallback
      return {} as T;
    } catch (error) {
      if (error instanceof PicoAPIError) {
        throw error;
      }
      throw new PicoAPIError(
        error instanceof Error ? error.message : 'Unknown error',
        undefined,
        error
      );
    }
  }

  // === Status Endpoints ===

  async getStatus(): Promise<KilnStatus> {
    return this.request<KilnStatus>('/api/status');
  }

  async getTuningStatus(): Promise<KilnStatus> {
    // Same as getStatus - the tuning info is included in the status response
    return this.request<KilnStatus>('/api/tuning/status');
  }

  // === Control Endpoints ===

  async runProfile(profileName: string): Promise<RunProfileResponse> {
    const body: RunProfileRequest = { profile: profileName };
    return this.request<RunProfileResponse>('/api/run', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
  }

  async stopProfile(): Promise<StopResponse> {
    return this.request<StopResponse>('/api/stop', {
      method: 'POST',
    });
  }

  async shutdown(): Promise<ShutdownResponse> {
    return this.request<ShutdownResponse>('/api/shutdown', {
      method: 'POST',
    });
  }

  // === Tuning Endpoints ===

  async startTuning(
    mode: StartTuningRequest['mode'],
    maxTemp?: number
  ): Promise<StartTuningResponse> {
    const body: StartTuningRequest = { mode, max_temp: maxTemp };
    return this.request<StartTuningResponse>('/api/tuning/start', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
  }

  async stopTuning(): Promise<StopTuningResponse> {
    return this.request<StopTuningResponse>('/api/tuning/stop', {
      method: 'POST',
    });
  }

  // === Scheduling Endpoints ===

  async scheduleProfile(
    profileName: string,
    startTime: number
  ): Promise<ScheduleProfileResponse> {
    const body: ScheduleProfileRequest = { 
      profile: profileName, 
      start_time: startTime 
    };
    return this.request<ScheduleProfileResponse>('/api/schedule', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
  }

  async getScheduledStatus(): Promise<ScheduledStatusResponse> {
    return this.request<ScheduledStatusResponse>('/api/scheduled');
  }

  async cancelScheduled(): Promise<CancelScheduledResponse> {
    return this.request<CancelScheduledResponse>('/api/scheduled/cancel', {
      method: 'POST',
    });
  }

  // === Helper Methods ===

  /**
   * Test connection to the Pico
   * Returns true if connection is successful, false otherwise
   */
  async testConnection(): Promise<boolean> {
    try {
      await this.getStatus();
      return true;
    } catch {
      return false;
    }
  }

  /**
   * Update the base URL for this client
   */
  setBaseURL(baseURL: string): void {
    this.baseURL = baseURL.replace(/\/$/, '');
  }

  /**
   * Get the current base URL
   */
  getBaseURL(): string {
    return this.baseURL;
  }
}
