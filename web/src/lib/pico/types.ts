// Type definitions for the Pico Kiln API

export type KilnState = "IDLE" | "RUNNING" | "TUNING" | "COMPLETE" | "ERROR";
export type TempUnits = "c" | "f";
export type TuningMode = "SAFE" | "STANDARD" | "THOROUGH" | "HIGH_TEMP";
export type ProfileStepType = "ramp" | "hold" | "cooling";

// Profile structure
export interface ProfileStep {
	type: ProfileStepType;
	target_temp?: number; // Optional for cooling steps without target
	desired_rate?: number; // For ramp steps
	min_rate?: number; // For ramp steps
	duration?: number; // For hold steps (in seconds)
}

export interface Profile {
	name: string;
	temp_units: TempUnits;
	description: string;
	steps: ProfileStep[];
}

// PID Statistics (from status endpoint)
export interface PIDStats {
	kp?: number;
	ki?: number;
	kd?: number;
	p_term?: number;
	i_term?: number;
	d_term?: number;
	output?: number;
}

// Tuning Information (when in TUNING state)
export interface TuningInfo {
	mode: TuningMode;
	max_temp: number;
	phase?: string;
	progress?: number;
	estimated_time_remaining?: number;
	oscillation_count?: number;
}

// Scheduled Profile Information
export interface ScheduledProfile {
	profile_filename: string;
	start_time: number; // Unix timestamp
	start_time_iso: string; // ISO 8601 string
	seconds_until_start: number;
}

// Main status response from GET /api/status
export interface KilnStatus {
	state: KilnState;
	current_temp: number;
	target_temp?: number;
	ssr_on: boolean;
	ssr_output?: number; // SSR duty cycle percentage (0-100)
	timestamp?: number;
	elapsed?: number; // Elapsed time in seconds
	step_elapsed?: number; // Elapsed time within current step in seconds

	// PID information
	pid?: PIDStats;

	// Profile information (when RUNNING) - at root level
	profile_name?: string; // Name of active profile
	step_index?: number; // Current step index (0-based)
	total_steps?: number; // Total number of steps in profile
	step_name?: string; // Current step type ('ramp' or 'hold')

	// Tuning information (when TUNING)
	tuning?: TuningInfo;

	// Scheduled profile information
	scheduled_profile?: ScheduledProfile;

	// Error information (when ERROR)
	error_message?: string;
	error?: string; // Alternative error field name

	// Rate control information
	measured_rate?: number; // Measured heating rate in °C/h
	desired_rate?: number; // Target rate for current step in °C/h

	// Recovery mode information
	is_recovering?: boolean; // True if in recovery mode
	recovery_target_temp?: number; // Target temp for recovery (°C)
}

// API Request/Response types

export interface RunProfileRequest {
	profile: string; // Profile name (without .json extension)
}

export interface RunProfileResponse {
	success: boolean;
	message?: string;
	error?: string;
}

export interface StopResponse {
	success: boolean;
	message?: string;
	error?: string;
}

export interface ShutdownResponse {
	success: boolean;
	message?: string;
	error?: string;
}

export interface StartTuningRequest {
	mode: TuningMode;
	max_temp?: number; // Optional, uses mode default if not provided
}

export interface StartTuningResponse {
	success: boolean;
	message?: string;
	error?: string;
}

export interface StopTuningResponse {
	success: boolean;
	message?: string;
	error?: string;
}

// Scheduling API types
export interface ScheduleProfileRequest {
	profile: string; // Profile name (without .json extension)
	start_time: number; // Unix timestamp
}

export interface ScheduleProfileResponse {
	success: boolean;
	message?: string;
	error?: string;
}

export interface ScheduledStatusResponse {
	scheduled: boolean;
	profile?: string;
	start_time?: number;
	start_time_iso?: string;
	seconds_until_start?: number;
}

export interface CancelScheduledResponse {
	success: boolean;
	message?: string;
	error?: string;
}

// Connection health
export interface ConnectionHealth {
	connected: boolean;
	lastSuccessfulRequest?: number; // timestamp
	consecutiveFailures: number;
	lastError?: string;
}

// File Management API types
export type FileDirectory = "profiles" | "logs" | "diag";

export interface FileMetadata {
	name: string;
	size: number;
	modified: number; // Unix timestamp
}

export interface ListFilesResponse {
	success: boolean;
	directory: FileDirectory;
	count: number;
	files: FileMetadata[];
	error?: string;
}

export interface DeleteFileResponse {
	success: boolean;
	message?: string;
	error?: string;
}

export interface UploadFileResponse {
	success: boolean;
	message?: string;
	filename?: string;
	error?: string;
}

// === Kiln Configuration (GET/POST /api/config) ===

export type ThermocoupleType =
	| "B"
	| "E"
	| "J"
	| "K"
	| "N"
	| "R"
	| "S"
	| "T"
	| "G8"
	| "G32";

/**
 * Full kiln configuration as served by GET /api/config. Field names are
 * UPPER_SNAKE_CASE to match the firmware wire format exactly. LCD_* keys may be
 * absent in the response when the LCD is not configured; the client fills them
 * with firmware defaults for editing (see lib/config/schema.ts LCD_DEFAULTS).
 */
export interface KilnConfig {
	// Hardware (GPIO / SPI)
	MAX31856_SPI_ID: number;
	MAX31856_SCK_PIN: number;
	MAX31856_MOSI_PIN: number;
	MAX31856_MISO_PIN: number;
	MAX31856_CS_PIN: number;
	SSR_PIN: number[];

	// Network
	WIFI_SSID: string;
	WIFI_PASSWORD: string;
	WIFI_STATIC_IP: string | null;
	WIFI_SUBNET: string | null;
	WIFI_GATEWAY: string | null;
	WIFI_DNS: string | null;
	WEB_SERVER_HOST: string;
	WEB_SERVER_PORT: number;

	// Temperature & sensor
	THERMOCOUPLE_TYPE: ThermocoupleType;
	TEMP_UNITS: TempUnits;
	THERMOCOUPLE_OFFSET: number;
	MAINS_FREQUENCY: number;
	THERMOCOUPLE_AVERAGING: number;
	TEMP_MEDIAN_WINDOW: number;

	// Control loop timing
	TEMP_READ_INTERVAL: number;
	PID_UPDATE_INTERVAL: number;
	STATUS_UPDATE_INTERVAL: number;
	SSR_UPDATE_INTERVAL: number;

	// PID + thermal model
	PID_KP_BASE: number;
	PID_KI_BASE: number;
	PID_KD_BASE: number;
	THERMAL_H: number;
	THERMAL_T_AMBIENT: number;

	// SSR power
	SSR_CYCLE_TIME: number;
	SSR_STAGGER_DELAY: number;

	// Safety + rate/stall
	MAX_TEMP: number;
	STALL_CHECK_INTERVAL: number;
	STALL_CONSECUTIVE_FAILS: number;
	STALL_MIN_STEP_TIME: number;
	RATE_MEASUREMENT_WINDOW: number;
	RATE_RECORDING_INTERVAL: number;
	MAX_RECOVERY_TEMP_DELTA: number;

	// Logging + watchdog
	LOGGING_INTERVAL: number;
	ENABLE_WATCHDOG: boolean;
	WATCHDOG_TIMEOUT: number;

	// LCD (optional)
	LCD_I2C_ID: number;
	LCD_I2C_SCL: number;
	LCD_I2C_SDA: number;
	LCD_I2C_FREQ: number;
	LCD_I2C_ADDR: number;
}

/** A single config value as held by the form. */
export type ConfigValue = string | number | boolean | number[] | null;

export interface SaveConfigResponse {
	success: boolean;
	message?: string;
	error?: string;
}
