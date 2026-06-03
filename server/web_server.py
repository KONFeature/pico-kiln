# server/web_server.py
# HTTP server for monitoring and control interface
#
# Runs on Core 2 via asyncio.start_server and talks to the control thread
# (Core 1) through thread-safe queues. It never touches hardware directly.
#
# Native asyncio design notes:
# - asyncio.start_server() owns the listening socket and the accept loop, so
#   there is no hand-rolled polling, no EAGAIN handling and no socket-restart
#   bookkeeping here.
# - Each connection is driven by a (reader, writer) Stream pair. All socket I/O
#   is awaited, so a slow client never blocks the Core 2 event loop (status
#   receiver, LCD, WiFi monitor keep running).

import asyncio
import json
import gc
import os
import time
import machine
from micropython import const
import config
from kiln.comms import CommandMessage, QueueHelper
from kiln.tuner import MODE_SAFE, MODE_STANDARD, MODE_THOROUGH, MODE_HIGH_TEMP
from server.status_receiver import get_status_receiver
from server.profile_cache import get_profile_cache
from server.html_cache import get_html_cache

# MEMORY OPTIMIZED: pre-encoded HTTP status lines (bytes, allocated once).
HTTP_200 = b"HTTP/1.1 200 OK\r\n"
HTTP_400 = b"HTTP/1.1 400 Bad Request\r\n"
HTTP_403 = b"HTTP/1.1 403 Forbidden\r\n"
HTTP_404 = b"HTTP/1.1 404 Not Found\r\n"
HTTP_405 = b"HTTP/1.1 405 Method Not Allowed\r\n"
HTTP_408 = b"HTTP/1.1 408 Request Timeout\r\n"
HTTP_413 = b"HTTP/1.1 413 Payload Too Large\r\n"
HTTP_500 = b"HTTP/1.1 500 Internal Server Error\r\n"

# Common headers (pre-encoded as bytes)
HEADER_CONTENT_TYPE_JSON = b"Content-Type: application/json\r\n"
HEADER_CONTENT_TYPE_HTML = b"Content-Type: text/html\r\n"
HEADER_CONTENT_TYPE_TEXT = b"Content-Type: text/plain\r\n"
# CORS headers to allow web app from different origin
HEADER_CORS = b"Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, PUT, DELETE, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\n"
HEADER_CONNECTION_CLOSE = b"Connection: close\r\n\r\n"

# Lookup tables built once at import (not rebuilt per response).
_STATUS_LINES = {
    200: HTTP_200,
    400: HTTP_400,
    403: HTTP_403,
    404: HTTP_404,
    405: HTTP_405,
    408: HTTP_408,
    413: HTTP_413,
    500: HTTP_500,
}
_CONTENT_TYPES = {
    'application/json': HEADER_CONTENT_TYPE_JSON,
    'text/html': HEADER_CONTENT_TYPE_HTML,
    'text/plain': HEADER_CONTENT_TYPE_TEXT,
}

# Global communication channels (initialized in start_server)
command_queue = None

# Module-level constants for connection and request limits
# Kept below lwIP's hard cap of 5 active TCP PCBs (MEMP_NUM_TCP_PCB, lwIP default,
# not overridden in the rp2 port). 3 = a file transfer + a status poll + 1 spare,
# with slack left for TIME_WAIT churn so we never hit lwIP-level connection drops.
MAX_CONCURRENT_CONNECTIONS = const(3)
MAX_UPLOAD_SIZE = const(512000)            # 500KB max upload (long profiles / tuning logs)
MAX_JSON_BODY = const(4096)                # Cap buffered (non-upload) request bodies; JSON commands are tiny
FILE_CHUNK_SIZE = const(1024)              # 1 KiB streaming chunk (Microdot-validated)
FILE_TRANSFER_TIMEOUT = const(60)          # Max seconds for one file transfer before its slot is reclaimed

# Validation tables built once at import (not rebuilt per request).
VALID_DIRECTORIES = ('profiles', 'logs')
VALID_TUNING_MODES = (MODE_SAFE, MODE_STANDARD, MODE_THOROUGH, MODE_HIGH_TEMP)

# Connection tracking
active_connections = 0


async def _close(writer):
    """Close a connection's writer, swallowing disconnect errors."""
    try:
        writer.close()
        await writer.wait_closed()
    except Exception:
        pass


async def _read_request(reader):
    """Read the request line and headers from a stream.

    Reads line-by-line so headers spanning multiple TCP segments are parsed
    correctly. Only Content-Length is kept (the one header any handler needs);
    the rest are scanned past, avoiding a throwaway per-request dict. The body
    is left in the stream for the handler to consume.

    Returns (method, path, content_length), or (None, None, 0) on empty/garbage.
    """
    request_line = await reader.readline()
    if not request_line:
        return None, None, 0

    try:
        parts = request_line.decode().split(' ')
        method = parts[0]
        path = parts[1] if len(parts) > 1 else '/'
    except Exception:
        return None, None, 0

    content_length = 0
    while True:
        line = await reader.readline()
        if not line or line == b'\r\n':
            break
        # "Content-Length:" is 15 bytes; match case-insensitively without decoding.
        if line[:15].lower() == b'content-length:':
            try:
                content_length = int(line[15:].strip())
            except Exception:
                content_length = 0

    return method, path, content_length


async def send_response(writer, status, body=b'', content_type='text/plain'):
    """Send an HTTP response.

    MEMORY OPTIMIZED: writes pre-encoded headers and the body as separate
    buffered writes, then a single drain(). Avoids the headers+body
    concatenation (one fewer allocation than the old sendall path).
    """
    writer.write(_STATUS_LINES.get(status, HTTP_500))
    writer.write(_CONTENT_TYPES.get(content_type, HEADER_CONTENT_TYPE_TEXT))
    writer.write(HEADER_CORS)
    writer.write(HEADER_CONNECTION_CLOSE)
    if body:
        writer.write(body)
    await writer.drain()


async def send_json_response(writer, data, status=200):
    """Send JSON response"""
    await send_response(writer, status, json.dumps(data).encode(), 'application/json')


async def send_html_response(writer, html, status=200):
    """Send HTML response"""
    await send_response(writer, status, html.encode() if isinstance(html, str) else html, 'text/html')


async def _send_command(writer, command, ok_message, ok_log=None):
    """Queue a bodyless control command with a uniform success / queue-full reply."""
    if QueueHelper.put_nowait(command_queue, command):
        if ok_log:
            print(ok_log)
        await send_json_response(writer, {'success': True, 'message': ok_message})
    else:
        print("[Web Server] Command queue full; command dropped")
        await send_json_response(writer, {'success': False, 'error': 'Command queue full, please retry'}, 500)


# === API Handlers ===

async def handle_api_shutdown(writer):
    """POST /api/shutdown - Emergency shutdown: turn off SSR and stop program"""
    await _send_command(
        writer,
        CommandMessage.shutdown(),
        'System shutdown: SSR off, program stopped',
        '[Web Server] Emergency shutdown triggered via API',
    )

# === Control Command Handlers ===

async def handle_api_run(writer, body):
    """POST /api/run - Start running a profile"""
    try:
        data = json.loads(body.decode())
        profile_name = data.get('profile')

        if not profile_name:
            await send_json_response(writer, {'success': False, 'error': 'Profile name required'}, 400)
            return

        # PERFORMANCE: Verify profile exists using cache instead of blocking filesystem check
        if not get_profile_cache().exists(profile_name):
            await send_json_response(writer, {'success': False, 'error': f'Profile not found: {profile_name}'}, 404)
            return

        # Send command to control thread (Core 1 will load the profile from disk)
        profile_filename = f"{profile_name}.json"
        command = CommandMessage.run_profile(profile_filename)

        if QueueHelper.put_nowait(command_queue, command):
            print(f"[Web Server] Started profile: {profile_name}")
            await send_json_response(writer, {
                'success': True,
                'message': f'Started profile: {profile_name}'
            })
        else:
            print("[Web Server] Failed to send run_profile command (queue full)")
            await send_json_response(writer, {
                'success': False,
                'error': 'Command queue full, please retry'
            }, 500)

    except Exception as e:
        print(f"[Web Server] Error starting profile: {e}")
        await send_json_response(writer, {'success': False, 'error': str(e)}, 400)

async def handle_api_stop(writer):
    """POST /api/stop - Stop current profile"""
    await _send_command(writer, CommandMessage.stop(), 'Profile stopped',
                        '[Web Server] Profile stop requested')

async def handle_api_clear_error(writer):
    """POST /api/clear-error - Clear error state and return to idle"""
    await _send_command(writer, CommandMessage.clear_error(), 'Error cleared, returned to idle',
                        '[Web Server] Clear error requested')

async def handle_api_reboot(writer):
    """POST /api/reboot - Reboot the Pico"""
    print("[Web Server] Reboot requested via API")

    # Send success response (send_response drains the socket buffer) before rebooting.
    await send_json_response(writer, {
        'success': True,
        'message': 'Rebooting Pico...'
    })

    # Give the client time to receive the response before the reset tears down TCP.
    await asyncio.sleep(0.5)

    # Reboot the device
    machine.reset()

async def handle_api_schedule(writer, body):
    """POST /api/schedule - Schedule profile for delayed start"""
    try:
        data = json.loads(body.decode())
        profile_name = data.get('profile')
        start_time = data.get('start_time')  # Unix timestamp

        if not profile_name or not start_time:
            await send_json_response(writer, {
                'success': False,
                'error': 'profile and start_time required'
            }, 400)
            return

        # Validate start time is in future
        if start_time <= time.time():
            await send_json_response(writer, {
                'success': False,
                'error': 'start_time must be in the future'
            }, 400)
            return

        # Check profile exists
        if not get_profile_cache().exists(profile_name):
            await send_json_response(writer, {
                'success': False,
                'error': f'Profile not found: {profile_name}'
            }, 404)
            return

        profile_filename = f"{profile_name}.json"
        command = CommandMessage.schedule_profile(profile_filename, start_time)

        if QueueHelper.put_nowait(command_queue, command):
            print(f"[Web Server] Scheduled profile: {profile_name}")
            await send_json_response(writer, {
                'success': True,
                'message': f'Scheduled profile: {profile_name}'
            })
        else:
            await send_json_response(writer, {
                'success': False,
                'error': 'Command queue full'
            }, 500)

    except Exception as e:
        print(f"[Web Server] Error scheduling profile: {e}")
        await send_json_response(writer, {'success': False, 'error': str(e)}, 400)

async def handle_api_scheduled_status(writer):
    """GET /api/scheduled - Get status of scheduled profile"""
    status = get_status_receiver().get_status()
    scheduled = status.get('scheduled_profile')

    if scheduled:
        await send_json_response(writer, {
            'scheduled': True,
            'profile': scheduled['profile_filename'],
            'start_time': scheduled['start_time'],
            'start_time_iso': scheduled['start_time_iso'],
            'seconds_until_start': scheduled['seconds_until_start']
        })
    else:
        await send_json_response(writer, {'scheduled': False})

async def handle_api_cancel_scheduled(writer):
    """POST /api/scheduled/cancel - Cancel scheduled profile"""
    await _send_command(writer, CommandMessage.cancel_scheduled(), 'Cancelled scheduled profile',
                        '[Web Server] Cancelled scheduled profile')

# === File Management Helpers ===

def check_idle_state():
    """
    Check if kiln is in IDLE state before file operations

    Returns:
        (is_idle, error_response) tuple
        - If idle: (True, None)
        - If not idle: (False, error_dict)
    """
    status = get_status_receiver().get_status()
    state = status.get('state', 'UNKNOWN')

    if state != 'IDLE':
        return False, {
            'success': False,
            'error': f'File operations not allowed while kiln is {state}. Stop the kiln first.'
        }

    return True, None

def validate_directory(directory):
    """
    Validate that directory is either 'profiles' or 'logs'

    Returns:
        (is_valid, path, error_response) tuple
    """
    if directory not in VALID_DIRECTORIES:
        return False, None, {
            'success': False,
            'error': "Invalid directory. Must be 'profiles' or 'logs'"
        }

    return True, directory, None

def safe_filename(filename):
    """
    Validate filename to prevent directory traversal

    Returns:
        True if safe, False otherwise
    """
    # Disallow path separators and parent directory references
    if '/' in filename or '\\' in filename or '..' in filename:
        return False

    # Must have some content
    if not filename or filename.startswith('.'):
        return False

    return True


def _remove_quietly(filepath):
    try:
        os.remove(filepath)
    except OSError:
        pass


async def _file_guard(writer, directory, filename=None):
    """Run the shared file-op preconditions.

    Returns the validated directory path, or None after sending the matching
    error: 403 if the kiln is not IDLE, 400 for a bad directory or unsafe
    filename. Pass filename=None for directory-only operations.
    """
    is_idle, error = check_idle_state()
    if not is_idle:
        await send_json_response(writer, error, 403)
        return None

    is_valid, dir_path, error = validate_directory(directory)
    if not is_valid:
        await send_json_response(writer, error, 400)
        return None

    if filename is not None and not safe_filename(filename):
        await send_json_response(writer, {'success': False, 'error': 'Invalid filename'}, 400)
        return None

    return dir_path

# === File Management Handlers ===

async def handle_api_files_list(writer, directory):
    """GET /api/files/<directory> - List files in directory"""
    dir_path = await _file_guard(writer, directory)
    if dir_path is None:
        return

    try:
        files = []
        for filename in os.listdir(dir_path):
            filepath = f"{dir_path}/{filename}"
            try:
                stat = os.stat(filepath)
                files.append({'name': filename, 'size': stat[6], 'modified': stat[8]})
            except:
                # If stat fails, just add name
                files.append({'name': filename, 'size': 0, 'modified': 0})

        await send_json_response(writer, {
            'success': True,
            'directory': directory,
            'files': files,
            'count': len(files)
        })
    except OSError as e:
        print(f"[Web Server] Error listing files: {e}")
        await send_json_response(writer, {'success': False, 'error': f'Failed to list directory: {e}'}, 500)

async def handle_api_files_get(writer, directory, filename):
    """GET /api/files/<directory>/<filename> - stream raw file content in 1KB chunks."""
    dir_path = await _file_guard(writer, directory, filename)
    if dir_path is None:
        return

    filepath = f"{dir_path}/{filename}"
    try:
        size = os.stat(filepath)[6]
    except OSError:
        await send_response(writer, 404, b'File not found', 'text/plain')
        return

    if filename.endswith('.csv'):
        content_type = b'text/csv'
    elif filename.endswith('.json'):
        content_type = b'application/json'
    else:
        content_type = b'text/plain'

    header = (
        HTTP_200 +
        b'Content-Type: ' + content_type + b'\r\n' +
        b'Content-Length: ' + str(size).encode() + b'\r\n' +
        b'Content-Disposition: attachment; filename="' + filename.encode() + b'"\r\n' +
        HEADER_CORS +
        HEADER_CONNECTION_CLOSE
    )

    # Reusable 1KB buffer keeps peak RAM flat regardless of file size.
    gc.collect()
    buf = bytearray(FILE_CHUNK_SIZE)
    mv = memoryview(buf)
    try:
        writer.write(header)
        await writer.drain()
        with open(filepath, 'rb') as f:
            while True:
                n = f.readinto(buf)
                if not n:
                    break
                writer.write(mv[:n])
                await writer.drain()
    except OSError as e:
        # A client dropping the connection mid-stream (navigate-away, refetch,
        # timeout) is routine, not an error: stay quiet on the disconnect errnos
        # and only surface genuinely unexpected failures. The timeout's
        # CancelledError is a BaseException and propagates past this handler.
        if (e.args[0] if e.args else None) not in (104, 32, 113, 5):  # ECONNRESET/EPIPE/ECONNABORTED/EIO
            print(f"[Web Server] Download {filename} error: {e}")
    except Exception as e:
        print(f"[Web Server] Download {filename} error: {e}")
    finally:
        gc.collect()

async def handle_api_files_delete(writer, directory, filename):
    """DELETE /api/files/<directory>/<filename> - Delete a file"""
    dir_path = await _file_guard(writer, directory, filename)
    if dir_path is None:
        return

    filepath = f"{dir_path}/{filename}"
    try:
        os.remove(filepath)
        print(f"[Web Server] Deleted file: {filepath}")
        await send_json_response(writer, {'success': True, 'message': f'Deleted {filename}'})
    except OSError as e:
        await send_json_response(writer, {'success': False, 'error': f'Failed to delete file: {e}'}, 500)

async def handle_api_files_upload(writer, directory, filename, reader, content_length):
    """PUT /api/files/<directory>/<filename> - stream the raw request body to disk in 1KB chunks."""
    dir_path = await _file_guard(writer, directory, filename)
    if dir_path is None:
        return

    if content_length <= 0:
        await send_json_response(writer, {'success': False, 'error': 'Missing or invalid Content-Length'}, 400)
        return
    if content_length > MAX_UPLOAD_SIZE:
        await send_json_response(writer, {
            'success': False,
            'error': f'File too large: {content_length} bytes (max {MAX_UPLOAD_SIZE})'
        }, 413)
        return

    filepath = f"{dir_path}/{filename}"
    gc.collect()
    buf = bytearray(FILE_CHUNK_SIZE)
    mv = memoryview(buf)

    written = 0
    try:
        with open(filepath, 'wb') as f:
            # The stream's internal buffer already holds any body bytes that
            # arrived with the headers; readinto() returns those first.
            while written < content_length:
                to_read = content_length - written
                if to_read > FILE_CHUNK_SIZE:
                    to_read = FILE_CHUNK_SIZE
                n = await reader.readinto(mv[:to_read])
                if not n:
                    break
                f.write(mv[:n])
                written += n
    except asyncio.CancelledError:
        # Timeout cancel: drop the partial file, let the supervisor see the cancel.
        _remove_quietly(filepath)
        raise
    except Exception as e:
        _remove_quietly(filepath)
        try:
            await send_json_response(writer, {'success': False, 'error': f'Failed to write file: {e}'}, 500)
        except Exception:
            pass
        return

    gc.collect()

    if written < content_length:
        _remove_quietly(filepath)
        try:
            await send_json_response(writer, {'success': False, 'error': 'Upload incomplete: client stopped sending'}, 408)
        except Exception:
            pass
        return

    if directory == 'profiles':
        get_profile_cache().refresh()

    print(f"[Web Server] Uploaded file: {filepath} ({written} bytes)")
    await send_json_response(writer, {
        'success': True,
        'message': f'Uploaded {filename}',
        'filename': filename,
        'size': written
    })

async def handle_api_files_delete_all(writer, directory):
    """DELETE /api/files/<directory>/all - Delete all files in directory"""
    try:
        # Check if IDLE
        is_idle, error = check_idle_state()
        if not is_idle:
            await send_json_response(writer, error, 403)
            return

        # Only allow for logs directory
        if directory != 'logs':
            await send_json_response(writer, {
                'success': False,
                'error': 'Bulk delete only allowed for logs directory'
            }, 403)
            return

        # Delete all files
        try:
            deleted = []
            errors = []

            for filename in os.listdir(directory):
                filepath = f"{directory}/{filename}"
                try:
                    os.remove(filepath)
                    deleted.append(filename)
                except Exception as e:
                    errors.append(f"{filename}: {e}")

            print(f"[Web Server] Deleted {len(deleted)} files from {directory}")

            response = {
                'success': True,
                'deleted_count': len(deleted),
                'deleted_files': deleted
            }

            if errors:
                response['errors'] = errors

            await send_json_response(writer, response)

        except OSError as e:
            await send_json_response(writer, {
                'success': False,
                'error': f'Failed to delete files: {e}'
            }, 500)

    except Exception as e:
        print(f"[Web Server] Error deleting all files: {e}")
        await send_json_response(writer, {'success': False, 'error': str(e)}, 500)

# === Status Handlers ===

async def handle_api_status(writer):
    """GET /api/status - Get detailed kiln status with PID stats"""
    await send_response(writer, 200, get_status_receiver().get_status_json(), 'application/json')

# === Tuning Handlers ===

async def handle_api_tuning_start(writer, body):
    """POST /api/tuning/start - Start PID auto-tuning"""
    try:
        data = json.loads(body.decode())
        mode = data.get('mode', MODE_STANDARD)
        max_temp = data.get('max_temp')  # None = use mode default

        # Validate mode
        if mode not in VALID_TUNING_MODES:
            await send_json_response(writer, {
                'success': False,
                'error': f'Invalid mode. Must be one of: {", ".join(VALID_TUNING_MODES)}'
            }, 400)
            return

        # Validate max_temp if provided
        if max_temp is not None:
            if max_temp < 50 or max_temp > 500:
                await send_json_response(writer, {
                    'success': False,
                    'error': 'Maximum temperature must be between 50°C and 500°C'
                }, 400)
                return

        # Send tuning command to control thread
        command = CommandMessage.start_tuning(mode=mode, max_temp=max_temp)

        if QueueHelper.put_nowait(command_queue, command):
            print(f"[Web Server] Started tuning (mode: {mode}, max_temp: {max_temp}°C)")
            await send_json_response(writer, {
                'success': True,
                'message': f'Tuning started in {mode} mode'
            })
        else:
            await send_json_response(writer, {
                'success': False,
                'error': 'Command queue full, please retry'
            }, 500)

    except Exception as e:
        print(f"[Web Server] Error starting tuning: {e}")
        await send_json_response(writer, {'success': False, 'error': str(e)}, 400)

async def handle_api_tuning_stop(writer):
    """POST /api/tuning/stop - Stop PID auto-tuning"""
    await _send_command(writer, CommandMessage.stop_tuning(), 'Tuning stopped',
                        '[Web Server] Tuning stop requested')

async def handle_api_tuning_status(writer):
    """GET /api/tuning/status - Get tuning status"""
    # Same pre-encoded status bytes as /api/status (tuning info is included when TUNING).
    await send_response(writer, 200, get_status_receiver().get_status_json(), 'application/json')

async def handle_tuning_page(writer):
    """Serve tuning.html page"""
    # MEMORY OPTIMIZED: Force garbage collection before building large response
    gc.collect()

    # PERFORMANCE: Use cached HTML instead of blocking file I/O
    html = get_html_cache().get('tuning')

    if html:
        await send_html_response(writer, html)
    else:
        # Fallback: cache miss
        await send_response(writer, 404, b'Tuning page not found', 'text/plain')

# === Static File Handlers ===

async def handle_index(writer):
    """Serve pre-rendered index.html (profiles list already included)"""
    # PERFORMANCE: Use pre-rendered HTML from cache (no JSON building, no replacements)
    html = get_html_cache().get('index')

    if html:
        # Send pre-rendered HTML - client will fetch data via /api/status
        await send_html_response(writer, html)
    else:
        # Fallback: cache miss (shouldn't happen if preload succeeded)
        await send_response(writer, 500, b'HTML cache miss', 'text/plain')

# === Request Router ===

async def _supervise_transfer(coro):
    """Run a size-unbounded file transfer under a wall-clock cap.

    asyncio.wait_for reclaims the connection slot if a client stalls past
    FILE_TRANSFER_TIMEOUT: it cancels the transfer (raising CancelledError into
    the handler, which closes its file and drops any partial upload) and waits
    for it to unwind before we return. An external cancel of the connection
    propagates out untouched so handle_client can close cleanly.
    """
    try:
        await asyncio.wait_for(coro, FILE_TRANSFER_TIMEOUT)
    except asyncio.TimeoutError:
        print(f"[Web Server] File transfer exceeded {FILE_TRANSFER_TIMEOUT}s; slot reclaimed")


# Exact-path routes: path -> (required_method, handler, needs_body).
# required_method None = any method allowed; needs_body True = pass the request body.
_ROUTES = {
    '/': (None, handle_index, False),
    '/index.html': (None, handle_index, False),
    '/tuning': (None, handle_tuning_page, False),
    '/tuning.html': (None, handle_tuning_page, False),
    '/api/status': (None, handle_api_status, False),
    '/api/shutdown': (None, handle_api_shutdown, False),
    '/api/run': ('POST', handle_api_run, True),
    '/api/stop': ('POST', handle_api_stop, False),
    '/api/clear-error': ('POST', handle_api_clear_error, False),
    '/api/reboot': ('POST', handle_api_reboot, False),
    '/api/tuning/start': ('POST', handle_api_tuning_start, True),
    '/api/tuning/stop': ('POST', handle_api_tuning_stop, False),
    '/api/tuning/status': ('GET', handle_api_tuning_status, False),
    '/api/schedule': ('POST', handle_api_schedule, True),
    '/api/scheduled': ('GET', handle_api_scheduled_status, False),
    '/api/scheduled/cancel': ('POST', handle_api_cancel_scheduled, False),
}


async def _route_files(reader, writer, method, path, content_length):
    """Dispatch /api/files/<directory>[/<filename>] (the variable-tail routes)."""
    parts = path.split('/')
    if len(parts) == 4:
        # /api/files/<directory>
        directory = parts[3]
        if method == 'GET':
            await handle_api_files_list(writer, directory)
        else:
            await send_response(writer, 405, b'Method not allowed', 'text/plain')
    elif len(parts) == 5:
        # /api/files/<directory>/<filename>
        directory = parts[3]
        filename = parts[4]
        if filename == 'all':
            if method == 'DELETE':
                await handle_api_files_delete_all(writer, directory)
            else:
                await send_response(writer, 405, b'Method not allowed', 'text/plain')
        elif method == 'GET':
            # Size-unbounded download: supervised with a wall-clock timeout.
            await _supervise_transfer(handle_api_files_get(writer, directory, filename))
        elif method == 'PUT':
            # Size-unbounded upload: streamed straight from the socket to disk.
            await _supervise_transfer(handle_api_files_upload(writer, directory, filename, reader, content_length))
        elif method == 'DELETE':
            await handle_api_files_delete(writer, directory, filename)
        else:
            await send_response(writer, 405, b'Method not allowed', 'text/plain')
    else:
        await send_response(writer, 404, b'Not found', 'text/plain')


async def _route(reader, writer, method, path, body, content_length):
    """Dispatch a parsed request to its handler."""
    # CORS preflight: 200 for any path.
    if method == 'OPTIONS':
        await send_response(writer, 200, b'', 'text/plain')
        return

    route = _ROUTES.get(path)
    if route is not None:
        req_method, handler, needs_body = route
        if req_method is not None and method != req_method:
            await send_response(writer, 405, b'Method not allowed', 'text/plain')
        elif needs_body:
            await handler(writer, body)
        else:
            await handler(writer)
        return

    if path.startswith('/api/files/'):
        await _route_files(reader, writer, method, path, content_length)
        return

    await send_response(writer, 404, b'Not found', 'text/plain')


async def handle_client(reader, writer):
    """asyncio.start_server callback: handle one client connection end to end."""
    global active_connections

    # Cap concurrency: reject (and immediately close) beyond the limit so we
    # never hold more than MAX_CONCURRENT_CONNECTIONS handler buffers at once.
    if active_connections >= MAX_CONCURRENT_CONNECTIONS:
        await _close(writer)
        return

    active_connections += 1
    try:
        method, path, content_length = await _read_request(reader)
        if not method:
            return

        # Body policy:
        # - PUT file uploads are streamed straight to disk by the handler, so we
        #   pass the live reader through and DO NOT buffer the body here.
        # - Every other request: read (and thus drain) the declared body so the
        #   socket is clean before we close it. JSON handlers consume `body`;
        #   bodyless endpoints simply discard it.
        is_file_upload = method == 'PUT' and path.startswith('/api/files/')

        # Reject oversized non-upload bodies: buffering a hostile Content-Length
        # here could exhaust the shared heap (JSON commands are tiny).
        if content_length > MAX_JSON_BODY and not is_file_upload:
            await send_response(writer, 413, b'Body too large', 'text/plain')
            return

        body = b''
        if content_length > 0 and not is_file_upload:
            try:
                body = await reader.readexactly(content_length)
            except Exception:
                # Short/aborted body: handlers that need it return 400 on decode.
                body = b''

        await _route(reader, writer, method, path, body, content_length)

    except Exception as e:
        print(f"Error handling request: {e}")
        try:
            await send_response(writer, 500, f'Server error: {e}'.encode(), 'text/plain')
        except Exception:
            pass

    finally:
        active_connections -= 1
        await _close(writer)


async def start_server(cmd_queue):
    """
    Start the HTTP server using the native asyncio TCP server.

    Args:
        cmd_queue: ThreadSafeQueue for sending commands to control thread

    Note:
        Status updates are handled by StatusReceiver singleton, which should
        be initialized and started separately in main.py
    """
    global command_queue
    command_queue = cmd_queue

    host = getattr(config, 'WEB_SERVER_HOST', '0.0.0.0')
    print(f"[Web Server] Starting HTTP server on port {config.WEB_SERVER_PORT}")

    # asyncio.start_server owns the socket + accept loop. The outer loop only
    # re-establishes the listener if start_server itself fails (e.g. transient
    # bind error); WiFi reconnects are handled by wifi_manager.monitor.
    while True:
        try:
            server = await asyncio.start_server(handle_client, host, config.WEB_SERVER_PORT)
            print("[Web Server] HTTP server listening!")
            async with server:
                await server.wait_closed()
        except asyncio.CancelledError:
            print("[Web Server] Server task cancelled; stopping")
            raise
        except Exception as e:
            print(f"[Web Server] Server error: {e}; retrying in 1s")
            await asyncio.sleep(1)
