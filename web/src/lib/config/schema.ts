import type { ConfigValue, KilnConfig } from "@/lib/pico/types";
import {
	host,
	ipv4Optional,
	num,
	optionalStr,
	pinList,
	str,
	type Validator,
} from "./validators";

export type FieldType =
	| "number"
	| "text"
	| "password"
	| "select"
	| "switch"
	| "pinlist"
	| "ip";

export interface ConfigFieldDef {
	key: keyof KilnConfig;
	label: string;
	help: string;
	type: FieldType;
	unit?: string;
	/** Show a hex hint next to a number input (e.g. "0x27"). */
	hexHint?: boolean;
	options?: { value: string; label: string }[];
	/** For ip fields: empty string is serialized as null. */
	nullable?: boolean;
	validate?: Validator;
}

export interface ConfigSectionDef {
	id: string;
	title: string;
	description?: string;
	advanced?: boolean;
	fields: ConfigFieldDef[];
}

/**
 * Firmware defaults for the only keys GET /api/config may omit: the LCD_* block,
 * which the firmware emits solely when the LCD is enabled (see kiln-app
 * config.rs `write_json`). Every other key is always present in the response, so
 * listing it here would only duplicate a firmware constant and risk silent
 * drift. Used to fill the LCD section so it stays editable and an untouched
 * value produces no diff.
 */
const LCD_DEFAULTS: Pick<
	KilnConfig,
	"LCD_I2C_ID" | "LCD_I2C_SCL" | "LCD_I2C_SDA" | "LCD_I2C_FREQ" | "LCD_I2C_ADDR"
> = {
	LCD_I2C_ID: 0,
	LCD_I2C_SCL: 21,
	LCD_I2C_SDA: 20,
	LCD_I2C_FREQ: 100000,
	LCD_I2C_ADDR: 39,
};

const PIN_HELP = "Raspberry Pi Pico GPIO number (0-29).";

export const SECTIONS: ConfigSectionDef[] = [
	{
		id: "temperature",
		title: "Temperature & Sensor",
		description: "Thermocouple readout, calibration and filtering.",
		fields: [
			{
				key: "THERMOCOUPLE_TYPE",
				label: "Thermocouple type",
				help: "Alloy type of your thermocouple. Must match the physical probe. K is the most common for kilns.",
				type: "select",
				options: ["B", "E", "J", "K", "N", "R", "S", "T", "G8", "G32"].map(
					(v) => ({ value: v, label: v }),
				),
			},
			{
				key: "TEMP_UNITS",
				label: "Display units",
				help: "Units shown in the UI. The control loop always runs in Celsius internally.",
				type: "select",
				options: [
					{ value: "c", label: "Celsius (°C)" },
					{ value: "f", label: "Fahrenheit (°F)" },
				],
			},
			{
				key: "THERMOCOUPLE_OFFSET",
				label: "Calibration offset",
				help: "Added to every temperature reading to correct sensor bias. Leave at 0 unless you've calibrated against a reference.",
				type: "number",
				unit: "°C",
				validate: num(-50, 50),
			},
			{
				key: "MAINS_FREQUENCY",
				label: "Mains frequency",
				help: "Your AC line frequency. Used by the MAX31856 noise-rejection filter.",
				type: "number",
				unit: "Hz",
				options: [
					{ value: "50", label: "50 Hz" },
					{ value: "60", label: "60 Hz" },
				],
			},
			{
				key: "THERMOCOUPLE_AVERAGING",
				label: "Hardware averaging",
				help: "Number of samples the MAX31856 averages per reading. Higher is smoother but slower.",
				type: "number",
				unit: "samples",
				options: [1, 2, 4, 8, 16].map((v) => ({
					value: String(v),
					label: String(v),
				})),
			},
			{
				key: "TEMP_MEDIAN_WINDOW",
				label: "Median filter window",
				help: "Software median filter size used to reject single-sample spikes. 1 disables it.",
				type: "number",
				unit: "samples",
				validate: num(1, 15, true),
			},
		],
	},
	{
		id: "pid",
		title: "PID & Thermal Model",
		description:
			"Core control gains and the optional thermal feed-forward model.",
		fields: [
			{
				key: "PID_KP_BASE",
				label: "Proportional gain (Kp)",
				help: "Base proportional term. Drives output proportional to the current temperature error.",
				type: "number",
				validate: num(0, 10000),
			},
			{
				key: "PID_KI_BASE",
				label: "Integral gain (Ki)",
				help: "Base integral term. Eliminates steady-state error over time. Small values are normal.",
				type: "number",
				validate: num(0, 1000),
			},
			{
				key: "PID_KD_BASE",
				label: "Derivative gain (Kd)",
				help: "Base derivative term. Damps overshoot by reacting to the rate of error change.",
				type: "number",
				validate: num(0, 100000),
			},
			{
				key: "THERMAL_H",
				label: "Thermal efficiency (H)",
				help: "Feed-forward thermal-model coefficient. 0 disables the model and uses pure PID.",
				type: "number",
				validate: num(0, 10000),
			},
			{
				key: "THERMAL_T_AMBIENT",
				label: "Ambient temperature",
				help: "Room temperature assumed by the thermal model.",
				type: "number",
				unit: "°C",
				validate: num(-20, 60),
			},
		],
	},
	{
		id: "ssr",
		title: "SSR / Power Control",
		description: "Solid-state relay switching behaviour.",
		fields: [
			{
				key: "SSR_CYCLE_TIME",
				label: "Duty-cycle period",
				help: "Length of one SSR on/off PWM window. The PID output sets the on-fraction of this window.",
				type: "number",
				unit: "s",
				validate: num(1, 120),
			},
			{
				key: "SSR_STAGGER_DELAY",
				label: "Multi-relay stagger delay",
				help: "Delay between switching multiple SSRs on, to limit inrush current. Ignored with a single relay.",
				type: "number",
				unit: "s",
				validate: num(0, 5),
			},
		],
	},
	{
		id: "safety",
		title: "Safety & Stall Detection",
		description:
			"Hard limits and heating-rate sanity checks that trip the kiln to ERROR.",
		fields: [
			{
				key: "MAX_TEMP",
				label: "Maximum temperature",
				help: "Hard safety ceiling. Exceeding it immediately faults the kiln to ERROR and cuts power.",
				type: "number",
				unit: "°C",
				validate: num(100, 1400),
			},
			{
				key: "STALL_CHECK_INTERVAL",
				label: "Stall check interval",
				help: "How often the heating rate is evaluated against the target during a ramp.",
				type: "number",
				unit: "s",
				validate: num(10, 3600),
			},
			{
				key: "STALL_CONSECUTIVE_FAILS",
				label: "Stall fail count",
				help: "Number of consecutive failed rate checks before the kiln declares a stall error.",
				type: "number",
				unit: "checks",
				validate: num(1, 20, true),
			},
			{
				key: "STALL_MIN_STEP_TIME",
				label: "Stall grace period",
				help: "Minimum time into a step before stall detection is allowed to fire.",
				type: "number",
				unit: "s",
				validate: num(0, 7200),
			},
			{
				key: "STALL_RATE_RATIO",
				label: "Stall rate ratio",
				help: "Fallback minimum heating rate as a fraction of a step's target rate, used when the step sets no explicit min rate. E.g. 0.8 stalls below 80% of the target rate. Set 0 to disable the fallback check.",
				type: "number",
				unit: "× rate",
				validate: num(0, 1),
			},
			{
				key: "RATE_MEASUREMENT_WINDOW",
				label: "Rate measurement window",
				help: "Rolling window used to compute the average heating/cooling rate.",
				type: "number",
				unit: "s",
				validate: num(60, 7200),
			},
			{
				key: "RATE_RECORDING_INTERVAL",
				label: "Rate sample interval",
				help: "How often a temperature sample is recorded for the rate calculation.",
				type: "number",
				unit: "s",
				validate: num(1, 600),
			},
			{
				key: "MAX_RECOVERY_TEMP_DELTA",
				label: "Max recovery temp drop",
				help: "After a crash or power loss, recovery is declined if the kiln cooled more than this.",
				type: "number",
				unit: "°C",
				validate: num(0, 200),
			},
		],
	},
	{
		id: "timing",
		title: "Control Loop Timing",
		description:
			"Cadence of the control loop's sub-tasks. Defaults suit most kilns.",
		fields: [
			{
				key: "TEMP_READ_INTERVAL",
				label: "Temperature read interval",
				help: "Outer control-tick period: how often a new temperature is read.",
				type: "number",
				unit: "s",
				validate: num(0.1, 10),
			},
			{
				key: "PID_UPDATE_INTERVAL",
				label: "PID update interval",
				help: "How often the PID controller recomputes its output.",
				type: "number",
				unit: "s",
				validate: num(0.1, 10),
			},
			{
				key: "STATUS_UPDATE_INTERVAL",
				label: "Status broadcast interval",
				help: "How often the controller publishes status to the web UI.",
				type: "number",
				unit: "s",
				validate: num(1, 60),
			},
			{
				key: "SSR_UPDATE_INTERVAL",
				label: "SSR update interval",
				help: "Sub-tick period at which the SSR duty cycle is refreshed.",
				type: "number",
				unit: "s",
				validate: num(0.05, 5),
			},
		],
	},
	{
		id: "logging",
		title: "Logging & Watchdog",
		description: "Run logging and the optional hardware watchdog.",
		fields: [
			{
				key: "LOGGING_INTERVAL",
				label: "Log row interval",
				help: "How often a row is written to the run CSV log.",
				type: "number",
				unit: "s",
				validate: num(1, 600, true),
			},
			{
				key: "ENABLE_WATCHDOG",
				label: "Enable hardware watchdog",
				help: "Resets the Pico automatically if the firmware ever hangs. Recommended on.",
				type: "switch",
			},
			{
				key: "WATCHDOG_TIMEOUT",
				label: "Watchdog timeout",
				help: "How long the firmware can be unresponsive before the watchdog resets it.",
				type: "number",
				unit: "ms",
				validate: num(1000, 30000, true),
			},
		],
	},
	{
		id: "lcd",
		title: "Display (LCD)",
		description:
			"Optional I²C LCD1602. If your kiln has no LCD, leave these untouched — editing any value enables the display.",
		fields: [
			{
				key: "LCD_I2C_ID",
				label: "I²C bus id",
				help: "Which I²C peripheral the LCD is wired to (0 or 1).",
				type: "number",
				validate: num(0, 1, true),
			},
			{
				key: "LCD_I2C_SCL",
				label: "SCL pin",
				help: `I²C clock pin. ${PIN_HELP}`,
				type: "number",
				unit: "GPIO",
				validate: num(0, 29, true),
			},
			{
				key: "LCD_I2C_SDA",
				label: "SDA pin",
				help: `I²C data pin. ${PIN_HELP}`,
				type: "number",
				unit: "GPIO",
				validate: num(0, 29, true),
			},
			{
				key: "LCD_I2C_FREQ",
				label: "I²C frequency",
				help: "I²C bus clock speed. 100000 (100 kHz) is standard.",
				type: "number",
				unit: "Hz",
				validate: num(10000, 1000000, true),
			},
			{
				key: "LCD_I2C_ADDR",
				label: "I²C address",
				help: "LCD backpack I²C address as a decimal number. Common: 39 (0x27) or 63 (0x3F).",
				type: "number",
				hexHint: true,
				validate: num(0, 127, true),
			},
		],
	},
	{
		id: "hardware",
		title: "Advanced — Hardware (GPIO)",
		advanced: true,
		description:
			"Thermocouple SPI wiring and SSR pins. Wrong values break temperature reads or relay control. Only change if you rewired the board — fixable only by re-flashing over USB.",
		fields: [
			{
				key: "MAX31856_SPI_ID",
				label: "SPI bus id",
				help: "Which SPI peripheral the MAX31856 is wired to (0 or 1).",
				type: "number",
				validate: num(0, 1, true),
			},
			{
				key: "MAX31856_SCK_PIN",
				label: "SCK pin",
				help: `SPI clock pin. ${PIN_HELP}`,
				type: "number",
				unit: "GPIO",
				validate: num(0, 29, true),
			},
			{
				key: "MAX31856_MOSI_PIN",
				label: "MOSI pin",
				help: `SPI controller-to-sensor data pin. ${PIN_HELP}`,
				type: "number",
				unit: "GPIO",
				validate: num(0, 29, true),
			},
			{
				key: "MAX31856_MISO_PIN",
				label: "MISO pin",
				help: `SPI sensor-to-controller data pin. ${PIN_HELP}`,
				type: "number",
				unit: "GPIO",
				validate: num(0, 29, true),
			},
			{
				key: "MAX31856_CS_PIN",
				label: "Chip-select pin",
				help: `SPI chip-select for the MAX31856. ${PIN_HELP}`,
				type: "number",
				unit: "GPIO",
				validate: num(0, 29, true),
			},
			{
				key: "SSR_PIN",
				label: "SSR pin(s)",
				help: `Comma-separated GPIO pin(s) driving the relay(s), e.g. "15" or "15, 14". Up to 10. ${PIN_HELP}`,
				type: "pinlist",
				unit: "GPIO",
				validate: pinList(10),
			},
		],
	},
	{
		id: "network",
		title: "Advanced — Network",
		advanced: true,
		description:
			"WiFi credentials and web-server binding. A wrong SSID/password or static IP will drop the Pico off the network — recoverable only over USB.",
		fields: [
			{
				key: "WIFI_SSID",
				label: "WiFi network (SSID)",
				help: "Name of the WiFi network the Pico connects to.",
				type: "text",
				validate: str(64),
			},
			{
				key: "WIFI_PASSWORD",
				label: "WiFi password",
				help: "Password for the WiFi network. Stored in plaintext on the device.",
				type: "password",
				validate: optionalStr(64),
			},
			{
				key: "WIFI_STATIC_IP",
				label: "Static IP",
				help: "Optional fixed IPv4 for the Pico. Leave blank to use DHCP.",
				type: "ip",
				nullable: true,
				validate: ipv4Optional(),
			},
			{
				key: "WIFI_SUBNET",
				label: "Subnet mask",
				help: "Subnet mask for the static IP. Leave blank for DHCP.",
				type: "ip",
				nullable: true,
				validate: ipv4Optional(),
			},
			{
				key: "WIFI_GATEWAY",
				label: "Gateway",
				help: "Default gateway for the static IP. Leave blank for DHCP.",
				type: "ip",
				nullable: true,
				validate: ipv4Optional(),
			},
			{
				key: "WIFI_DNS",
				label: "DNS server",
				help: "DNS server for the static IP. Leave blank for DHCP.",
				type: "ip",
				nullable: true,
				validate: ipv4Optional(),
			},
			{
				key: "WEB_SERVER_HOST",
				label: "Server bind address",
				help: "Address the HTTP server binds to. 0.0.0.0 listens on all interfaces.",
				type: "text",
				validate: host(),
			},
			{
				key: "WEB_SERVER_PORT",
				label: "Server port",
				help: "TCP port for the web server. Default 80.",
				type: "number",
				validate: num(1, 65535, true),
			},
		],
	},
];

/** Flat list of every field def, in section order. */
export const ALL_FIELDS: ConfigFieldDef[] = SECTIONS.flatMap((s) => s.fields);

/** Fill the LCD_* keys the GET response omits when the LCD is disabled. */
export function withDefaults(raw: KilnConfig): KilnConfig {
	return { ...LCD_DEFAULTS, ...raw };
}

/**
 * Normalize a form value to its wire representation for a field: trims and nulls
 * empty ip strings, coerces numeric inputs/selects to numbers, leaves arrays and
 * booleans as-is.
 */
function normalize(def: ConfigFieldDef, value: ConfigValue): ConfigValue {
	if (def.type === "ip") {
		const s = typeof value === "string" ? value.trim() : "";
		return s.length === 0 ? null : s;
	}
	if (def.type === "number") {
		if (value === "" || value === null || value === undefined) return value;
		return typeof value === "number" ? value : Number(value);
	}
	return value;
}

function equal(a: ConfigValue, b: ConfigValue): boolean {
	if (Array.isArray(a) || Array.isArray(b)) {
		return JSON.stringify(a) === JSON.stringify(b);
	}
	return a === b;
}

/**
 * Build the sparse PATCH: only keys whose normalized form value differs from the
 * (default-filled) server config. LCD keys are included only when changed, so an
 * untouched LCD section never accidentally enables the display.
 */
export function buildPatch(
	values: Record<string, ConfigValue>,
	server: KilnConfig,
): Record<string, ConfigValue> {
	const patch: Record<string, ConfigValue> = {};
	for (const def of ALL_FIELDS) {
		const key = def.key as string;
		const next = normalize(def, values[key]);
		const base = normalize(def, server[def.key] as ConfigValue);
		if (!equal(next, base)) {
			patch[key] = next;
		}
	}
	return patch;
}
