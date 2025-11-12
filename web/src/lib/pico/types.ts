// Type definitions for the Pico Kiln API

export type KilnState = 'IDLE' | 'RUNNING' | 'TUNING' | 'ERROR';
export type TempUnits = 'c' | 'f';
export type TuningMode = 'safe' | 'standard' | 'thorough' | 'high_temp';
export type ProfileStepType = 'ramp' | 'hold';

// Profile structure
export interface ProfileStep {
  type: ProfileStepType;
  target_temp: number;
  desired_rate?: number; // For ramp steps
  min_rate?: number;     // For ramp steps
  duration?: number;     // For hold steps (in seconds)
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
  ssr_output?: number;    // SSR duty cycle percentage (0-100)
  timestamp?: number;
  elapsed?: number;       // Elapsed time in seconds
  
  // PID information
  pid?: PIDStats;
  
  // Profile information (when RUNNING) - at root level
  profile_name?: string;  // Name of active profile
  step_index?: number;    // Current step index (0-based)
  total_steps?: number;   // Total number of steps in profile
  step_name?: string;     // Current step type ('ramp' or 'hold')
  
  // Tuning information (when TUNING)
  tuning?: TuningInfo;
  
  // Scheduled profile information
  scheduled_profile?: ScheduledProfile;
  
  // Error information (when ERROR)
  error_message?: string;
  error?: string;         // Alternative error field name
  
  // Rate control information
  current_rate?: number;     // Adapted rate in °C/h
  actual_rate?: number;      // Measured rate in °C/h
  desired_rate?: number;     // Target rate for current step in °C/h
  adaptation_count?: number; // Number of rate adaptations made
  
  // Recovery mode information
  is_recovering?: boolean;           // True if in recovery mode
  recovery_target_temp?: number;     // Target temp for recovery (°C)
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
export type FileDirectory = 'profiles' | 'logs';

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

export interface GetFileResponse {
  success: boolean;
  filename: string;
  content: string;
  error?: string;
}

export interface DeleteFileResponse {
  success: boolean;
  message?: string;
  error?: string;
}

export interface DeleteAllFilesResponse {
  success: boolean;
  deleted_count: number;
  deleted_files: string[];
  error?: string;
}

export interface UploadFileResponse {
  success: boolean;
  message?: string;
  filename?: string;
  error?: string;
}
