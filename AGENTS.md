# AGENTS.md - Coding Agent Guidelines for pico-kiln

## Build/Deploy/Test Commands
- **MicroPython code (Pico 2)**: `./compile.sh` (dev) or `./compile.sh --production` (prod) → `./deploy.sh` or `./deploy.sh --clean`
- **Web frontend (React)**: `cd web && bun dev` (dev), `bun run build` (prod), `bun test` (run tests), `bun run check` (lint+format)
- **Python analysis scripts**: `python3 scripts/plot_run.py <csv>`, `python3 scripts/analyze_tuning.py <csv>`
- **Deploy profiles**: `./sync_profiles.sh`, **Clean logs**: `./clean_logs.sh`

## Code Style - MicroPython (Pico 2)

### Imports & Organization
- Group imports: stdlib → micropython → local modules (e.g., `time`, `micropython`, `kiln.state`)
- Use `from micropython import const` for constants (memory optimization)
- Path setup for lib: `sys.path.append('/lib')` when needed

### Formatting & Naming
- **Indentation**: 4 spaces (NOT tabs)
- **Naming**: snake_case for functions/variables, PascalCase for classes, UPPER_SNAKE for constants
- **Docstrings**: Required for classes and non-trivial functions (Google style with Args/Returns)
- **Comments**: Focus on WHY, not WHAT; use inline comments sparingly

### Performance & Types
- Use `@micropython.native` decorator for hot-path functions (called frequently, e.g., PID.update())
- No explicit type hints (MicroPython doesn't support them, but use clear variable names)
- Prefer const() for integer constants (compile-time optimization)

### Error Handling
- Use `try/except` with specific exceptions; avoid bare `except:`
- Print errors to console (no rich error logging on device due to memory constraints)
- Safety-critical code: validate inputs, handle sensor failures gracefully

## Code Style - Web Frontend (React/TypeScript)

### Formatting & Linting
- **Tool**: Biome (configured in web/biome.json)
- **Indentation**: TABS (not spaces), enforced by Biome
- **Quotes**: Double quotes for JavaScript/TypeScript
- **Imports**: Auto-organized by Biome assist actions

### TypeScript
- **Strict mode enabled** (tsconfig.json): All code must be fully typed
- **No unused vars/params** (enforced by compiler)
- Use path aliases: `@/*` maps to `./src/*`

## Additional Guidelines
- **Multi-threaded architecture**: Core 1 = control (temp, PID, SSR), Core 2 = web/WiFi/logging
- **Communication**: Thread-safe queues between cores; respect quiet mode during boot
- **Memory constraints**: Pico 2 has limited RAM (~200KB free); avoid large buffers, prefer streaming
- **Deploy workflow**: Edit → compile.sh → deploy.sh → test on device
