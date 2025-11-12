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

// Profile Progress (when in RUNNING state)
export interface ProfileProgress {
  profile_name: string;
  current_step: number;
  total_steps: number;
  step_type: ProfileStepType;
  step_progress?: number;  // Percentage 0-100
  elapsed_time?: number;   // Seconds
  estimated_time_remaining?: number; // Seconds
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
  timestamp?: number;
  
  // PID information
  pid?: PIDStats;
  
  // Profile information (when RUNNING)
  profile?: ProfileProgress;
  
  // Tuning information (when TUNING)
  tuning?: TuningInfo;
  
  // Scheduled profile information
  scheduled_profile?: ScheduledProfile;
  
  // Error information (when ERROR)
  error_message?: string;
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
