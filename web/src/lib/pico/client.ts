// API client for communicating with the Pico Kiln controller

import type {
	CancelScheduledResponse,
	DeleteAllFilesResponse,
	DeleteFileResponse,
	FileDirectory,
	KilnConfig,
	KilnStatus,
	ListFilesResponse,
	RunProfileRequest,
	RunProfileResponse,
	SaveConfigResponse,
	ScheduledStatusResponse,
	ScheduleProfileRequest,
	ScheduleProfileResponse,
	ShutdownResponse,
	StartTuningRequest,
	StartTuningResponse,
	StopResponse,
	StopTuningResponse,
	UploadFileResponse,
} from "./types";

/**
 * Client-side fetch-abort deadline for file up/downloads (NOT a server-side
 * wait). Longer than the 10s default since a transfer outlasts a status poll,
 * but sized for the server's 500KB cap: ~500KB completes well under 60s even on
 * weak WiFi (needs only ~8.5KB/s), so a stalled transfer aborts promptly and
 * frees one of the Pico's 2 connection slots instead of lingering for minutes.
 */
const FILE_TRANSFER_TIMEOUT_MS = 60_000;

export class PicoAPIError extends Error {
	constructor(
		message: string,
		public statusCode?: number,
		public originalError?: unknown,
	) {
		super(message);
		this.name = "PicoAPIError";
	}
}

export class PicoAPIClient {
	private baseURL: string;
	private timeoutMs: number;

	constructor(baseURL: string, timeoutMs = 10000) {
		// Ensure baseURL doesn't end with a slash
		// Empty string means use relative URLs (for proxy mode)
		this.baseURL = baseURL ? baseURL.replace(/\/$/, "") : "";
		this.timeoutMs = timeoutMs;
	}

	private async fetchWithTimeout(
		url: string,
		options: RequestInit = {},
		timeoutMs: number = this.timeoutMs,
	): Promise<Response> {
		const controller = new AbortController();
		const timeoutId = setTimeout(() => controller.abort(), timeoutMs);

		try {
			const response = await fetch(url, {
				...options,
				signal: controller.signal,
			});
			clearTimeout(timeoutId);
			return response;
		} catch (error) {
			clearTimeout(timeoutId);
			if (error instanceof Error && error.name === "AbortError") {
				throw new PicoAPIError(
					`Request timeout after ${timeoutMs}ms`,
					undefined,
					error,
				);
			}
			throw new PicoAPIError("Network request failed", undefined, error);
		}
	}

	private async request<T>(
		endpoint: string,
		options: RequestInit = {},
		timeoutMs?: number,
	): Promise<T> {
		const url = `${this.baseURL}${endpoint}`;

		try {
			const response = await this.fetchWithTimeout(url, options, timeoutMs);

			if (!response.ok) {
				const errorText = await response.text().catch(() => "Unknown error");
				throw new PicoAPIError(
					`HTTP ${response.status}: ${errorText}`,
					response.status,
				);
			}

			const contentType = response.headers.get("content-type");
			if (contentType?.includes("application/json")) {
				return await response.json();
			}

			// Fail loudly: a non-JSON body would otherwise become `{} as T` and crash downstream.
			throw new PicoAPIError(
				"Received an unexpected (non-JSON) response from the kiln.",
				response.status,
			);
		} catch (error) {
			if (error instanceof PicoAPIError) {
				throw error;
			}
			throw new PicoAPIError(
				error instanceof Error ? error.message : "Unknown error",
				undefined,
				error,
			);
		}
	}

	// === Status Endpoints ===

	async getStatus(): Promise<KilnStatus> {
		return this.request<KilnStatus>("/api/status");
	}

	async getTuningStatus(): Promise<KilnStatus> {
		// Same as getStatus - the tuning info is included in the status response
		return this.request<KilnStatus>("/api/tuning/status");
	}

	// === Control Endpoints ===

	async runProfile(profileName: string): Promise<RunProfileResponse> {
		const body: RunProfileRequest = { profile: profileName };
		return this.request<RunProfileResponse>("/api/run", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(body),
		});
	}

	async stopProfile(): Promise<StopResponse> {
		return this.request<StopResponse>("/api/stop", {
			method: "POST",
		});
	}

	async shutdown(): Promise<ShutdownResponse> {
		return this.request<ShutdownResponse>("/api/shutdown", {
			method: "POST",
		});
	}

	async clearError(): Promise<{ success: boolean; message: string }> {
		return this.request("/api/clear-error", {
			method: "POST",
		});
	}

	async reboot(): Promise<{ success: boolean; message: string }> {
		return this.request("/api/reboot", {
			method: "POST",
		});
	}

	// === Config Endpoints ===

	/**
	 * Fetch the full kiln configuration. LCD_* keys may be omitted by the
	 * firmware when the LCD is disabled; callers fill defaults for editing.
	 */
	async getConfig(): Promise<KilnConfig> {
		return this.request<KilnConfig>("/api/config");
	}

	/**
	 * Persist a sparse config PATCH (only the changed keys). The firmware merges
	 * it over the running config and applies it on the next reboot. The body must
	 * stay under the firmware's 2 KiB limit — a diff always does.
	 */
	async saveConfig(
		patch: Record<string, unknown>,
	): Promise<SaveConfigResponse> {
		return this.request<SaveConfigResponse>("/api/config", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(patch),
		});
	}

	// === Tuning Endpoints ===

	async startTuning(
		mode: StartTuningRequest["mode"],
		maxTemp?: number,
	): Promise<StartTuningResponse> {
		const body: StartTuningRequest = { mode, max_temp: maxTemp };
		return this.request<StartTuningResponse>("/api/tuning/start", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(body),
		});
	}

	async stopTuning(): Promise<StopTuningResponse> {
		return this.request<StopTuningResponse>("/api/tuning/stop", {
			method: "POST",
		});
	}

	// === Scheduling Endpoints ===

	async scheduleProfile(
		profileName: string,
		startTime: number,
	): Promise<ScheduleProfileResponse> {
		const body: ScheduleProfileRequest = {
			profile: profileName,
			start_time: startTime,
		};
		return this.request<ScheduleProfileResponse>("/api/schedule", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(body),
		});
	}

	async getScheduledStatus(): Promise<ScheduledStatusResponse> {
		return this.request<ScheduledStatusResponse>("/api/scheduled");
	}

	async cancelScheduled(): Promise<CancelScheduledResponse> {
		return this.request<CancelScheduledResponse>("/api/scheduled/cancel", {
			method: "POST",
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
		this.baseURL = baseURL.replace(/\/$/, "");
	}

	/**
	 * Get the current base URL
	 */
	getBaseURL(): string {
		return this.baseURL;
	}

	// === File Management Endpoints ===

	/**
	 * List all files in a directory (profiles or logs)
	 * Only works when kiln is IDLE
	 */
	async listFiles(directory: FileDirectory): Promise<ListFilesResponse> {
		return this.request<ListFilesResponse>(`/api/files/${directory}`);
	}

	/**
	 * Get file content from a directory
	 * Only works when kiln is IDLE
	 */
	async getFile(directory: FileDirectory, filename: string): Promise<string> {
		// Raw streaming download from /api/files/<dir>/<file>: the server returns
		// the file body (text/csv or application/json) with no JSON wrapper,
		// avoiding ~4x RAM amplification on the Pico. Uses a generous but bounded
		// timeout instead of none, so a stalled transfer aborts client-side and
		// frees the Pico's (only 2) connection slots.
		const url = `${this.baseURL}/api/files/${directory}/${filename}`;
		const response = await this.fetchWithTimeout(
			url,
			{},
			FILE_TRANSFER_TIMEOUT_MS,
		);
		if (!response.ok) {
			const errorText = await response.text().catch(() => "Unknown error");
			throw new PicoAPIError(
				`HTTP ${response.status}: ${errorText}`,
				response.status,
			);
		}
		return await response.text();
	}

	/**
	 * Delete a single file
	 * Only works when kiln is IDLE
	 */
	async deleteFile(
		directory: FileDirectory,
		filename: string,
	): Promise<DeleteFileResponse> {
		return this.request<DeleteFileResponse>(
			`/api/files/${directory}/${filename}`,
			{
				method: "DELETE",
			},
		);
	}

	/**
	 * Delete all files in logs directory
	 * Only allowed for logs directory, only works when kiln is IDLE
	 */
	async deleteAllLogs(): Promise<DeleteAllFilesResponse> {
		return this.request<DeleteAllFilesResponse>("/api/files/logs/all", {
			method: "DELETE",
		});
	}

	/**
	 * Upload a file to the Pico
	 * Only works when kiln is IDLE
	 */
	async uploadFile(
		directory: FileDirectory,
		filename: string,
		content: string,
	): Promise<UploadFileResponse> {
		// The raw request body IS the file content (no JSON wrapper): the Pico
		// streams it straight to disk in 1KB chunks, keeping peak RAM ~1KB even
		// at the 500KB limit. Uses the generous file-transfer timeout, not the
		// 10s default, since large uploads over Pico WiFi can be slow.
		return this.request<UploadFileResponse>(
			`/api/files/${directory}/${filename}`,
			{
				method: "PUT",
				headers: { "Content-Type": "text/plain; charset=utf-8" },
				body: content,
			},
			FILE_TRANSFER_TIMEOUT_MS,
		);
	}
}
