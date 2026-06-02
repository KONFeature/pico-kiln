# server/web_server.py
# HTTP server for monitoring and control interface
#
# This module runs on Core 2 and communicates with the control thread (Core 1)
# via thread-safe queues. It never directly accesses hardware.

import asyncio
import json
import socket
import gc
from micropython import const
import config
from kiln.comms import CommandMessage, QueueHelper
from kiln.tuner import MODE_SAFE, MODE_STANDARD, MODE_THOROUGH, MODE_HIGH_TEMP
from server.status_receiver import get_status_receiver

# MEMORY OPTIMIZED: Pre-encoded HTTP headers to avoid f-string allocations
# Common status lines (pre-encoded as bytes)
HTTP_200 = b"HTTP/1.1 200 OK\r\n"
HTTP_404 = b"HTTP/1.1 404 Not Found\r\n"
HTTP_500 = b"HTTP/1.1 500 Internal Server Error\r\n"

# Common headers (pre-encoded as bytes)
HEADER_CONTENT_TYPE_JSON = b"Content-Type: application/json\r\n"
HEADER_CONTENT_TYPE_HTML = b"Content-Type: text/html\r\n"
HEADER_CONTENT_TYPE_TEXT = b"Content-Type: text/plain\r\n"
# CORS headers to allow web app from different origin
HEADER_CORS = b"Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, PUT, DELETE, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\n"
HEADER_CONNECTION_CLOSE = b"Connection: close\r\n\r\n"

# Global communication channels (initialized in start_server)
command_queue = None

# Module-level constants for connection and request limits
MAX_CONCURRENT_CONNECTIONS = const(2)      # Limit to 2 concurrent connections on Pico
MAX_UPLOAD_SIZE = const(512000)            # 500KB max upload (long profiles / tuning logs)
FILE_CHUNK_SIZE = const(1024)              # 1 KiB streaming chunk (Microdot-validated)
FILE_TRANSFER_TIMEOUT = const(60)          # Max seconds for one file transfer before its slot is reclaimed

# Performance: const() declarations for server loop timing
SERVER_LOOP_INTERVAL = 0.1  # 100ms between accept() calls
MAX_SOCKET_ERRORS = const(50)  # 5 seconds of errors at 100ms interval
MAX_SOCKET_ERROR_PRINT = const(3)  # Only print first N errors to reduce USB spam

# Connection tracking
active_connections = 0

def parse_request(data):
    """Parse HTTP request and return method, path, headers, and body"""
    lines = data.split(b'\r\n')
    request_line = lines[0].decode()
    parts = request_line.split(' ')
    method, path = parts[0], parts[1] if len(parts) > 1 else '/'

    # Find body
    body_start = data.find(b'\r\n\r\n')
    body = data[body_start + 4:] if body_start != -1 else b''

    # Parse headers
    headers = {}
    for line in lines[1:]:
        if line == b'':
            break
        try:
            key, value = line.decode().split(': ', 1)
            headers[key.lower()] = value
        except:
            pass

    return method, path, headers, body

def send_response(conn, status, body=b'', content_type='text/plain'):
    """Send HTTP response (MEMORY OPTIMIZED: uses pre-encoded headers)"""
    # Map status codes to pre-encoded status lines
    status_line = {200: HTTP_200, 404: HTTP_404, 500: HTTP_500}.get(status, HTTP_500)

    # Map content types to pre-encoded headers
    content_type_header = {
        'application/json': HEADER_CONTENT_TYPE_JSON,
        'text/html': HEADER_CONTENT_TYPE_HTML,
        'text/plain': HEADER_CONTENT_TYPE_TEXT
    }.get(content_type, HEADER_CONTENT_TYPE_TEXT)

    # MEMORY OPTIMIZED: Build headers as single bytes object, then send with body
    # This reduces from 4 separate send() calls to just 1 sendall() call
    # Include CORS headers to allow cross-origin requests from web app
    headers = status_line + content_type_header + HEADER_CORS + HEADER_CONNECTION_CLOSE
    conn.sendall(headers + body)

def send_json_response(conn, data, status=200):
    """Send JSON response"""
    json_data = json.dumps(data)
    send_response(conn, status, json_data.encode(), 'application/json')

def send_html_response(conn, html, status=200):
    """Send HTML response"""
    send_response(conn, status, html.encode() if isinstance(html, str) else html, 'text/html')


# === API Handlers ===

def handle_api_shutdown(conn):
    """POST /api/shutdown - Emergency shutdown: turn off SSR and stop program"""
    # Send shutdown command to control thread
    command = CommandMessage.shutdown()

    if QueueHelper.put_nowait(command_queue, command):
        print("[Web Server] Emergency shutdown triggered via API")
        response = {
            "success": True,
            "message": "System shutdown: SSR off, program stopped"
        }
        send_json_response(conn, response)
    else:
        print("[Web Server] Failed to send shutdown command (queue full)")
        send_json_response(conn, {
            "success": False,
            "error": "Command queue full, please retry"
        }, 500)

# === Control Command Handlers ===

def handle_api_run(conn, body):
    """POST /api/run - Start running a profile"""
    try:
        data = json.loads(body.decode())
        profile_name = data.get('profile')

        if not profile_name:
            send_json_response(conn, {'success': False, 'error': 'Profile name required'}, 400)
            return

        # PERFORMANCE: Verify profile exists using cache instead of blocking filesystem check
        from server.profile_cache import get_profile_cache
        if not get_profile_cache().exists(profile_name):
            send_json_response(conn, {'success': False, 'error': f'Profile not found: {profile_name}'}, 404)
            return

        # Send command to control thread (Core 1 will load the profile from disk)
        profile_filename = f"{profile_name}.json"
        command = CommandMessage.run_profile(profile_filename)

        if QueueHelper.put_nowait(command_queue, command):
            print(f"[Web Server] Started profile: {profile_name}")
            cancel_file_transfers()
            send_json_response(conn, {
                'success': True,
                'message': f'Started profile: {profile_name}'
            })
        else:
            print("[Web Server] Failed to send run_profile command (queue full)")
            send_json_response(conn, {
                'success': False,
                'error': 'Command queue full, please retry'
            }, 500)

    except Exception as e:
        print(f"[Web Server] Error starting profile: {e}")
        send_json_response(conn, {'success': False, 'error': str(e)}, 400)

def handle_api_stop(conn):
    """POST /api/stop - Stop current profile"""
    # Send stop command to control thread
    command = CommandMessage.stop()

    if QueueHelper.put_nowait(command_queue, command):
        print("[Web Server] Profile stop requested")
        send_json_response(conn, {'success': True, 'message': 'Profile stopped'})
    else:
        print("[Web Server] Failed to send stop command (queue full)")
        send_json_response(conn, {
            'success': False,
            'error': 'Command queue full, please retry'
        }, 500)

def handle_api_clear_error(conn):
    """POST /api/clear-error - Clear error state and return to idle"""
    # Send clear_error command to control thread
    command = CommandMessage.clear_error()

    if QueueHelper.put_nowait(command_queue, command):
        print("[Web Server] Clear error requested")
        send_json_response(conn, {'success': True, 'message': 'Error cleared, returned to idle'})
    else:
        print("[Web Server] Failed to send clear_error command (queue full)")
        send_json_response(conn, {
            'success': False,
            'error': 'Command queue full, please retry'
        }, 500)

def handle_api_reboot(conn):
    """POST /api/reboot - Reboot the Pico"""
    import machine
    
    print("[Web Server] Reboot requested via API")
    
    # Send success response before rebooting
    send_json_response(conn, {
        'success': True,
        'message': 'Rebooting Pico...'
    })
    
    # Give client time to receive response
    import time
    time.sleep(0.5)
    
    # Reboot the device
    machine.reset()

def handle_api_schedule(conn, body):
    """POST /api/schedule - Schedule profile for delayed start"""
    try:
        import time
        data = json.loads(body.decode())
        profile_name = data.get('profile')
        start_time = data.get('start_time')  # Unix timestamp
        
        if not profile_name or not start_time:
            send_json_response(conn, {
                'success': False, 
                'error': 'profile and start_time required'
            }, 400)
            return
        
        # Validate start time is in future
        if start_time <= time.time():
            send_json_response(conn, {
                'success': False,
                'error': 'start_time must be in the future'
            }, 400)
            return
        
        # Check profile exists
        from server.profile_cache import get_profile_cache
        if not get_profile_cache().exists(profile_name):
            send_json_response(conn, {
                'success': False, 
                'error': f'Profile not found: {profile_name}'
            }, 404)
            return
        
        profile_filename = f"{profile_name}.json"
        command = CommandMessage.schedule_profile(profile_filename, start_time)
        
        if QueueHelper.put_nowait(command_queue, command):
            print(f"[Web Server] Scheduled profile: {profile_name}")
            send_json_response(conn, {
                'success': True,
                'message': f'Scheduled profile: {profile_name}'
            })
        else:
            send_json_response(conn, {
                'success': False,
                'error': 'Command queue full'
            }, 500)
    
    except Exception as e:
        print(f"[Web Server] Error scheduling profile: {e}")
        send_json_response(conn, {'success': False, 'error': str(e)}, 400)

def handle_api_scheduled_status(conn):
    """GET /api/scheduled - Get status of scheduled profile"""
    status = get_status_receiver().get_status()
    scheduled = status.get('scheduled_profile')
    
    if scheduled:
        send_json_response(conn, {
            'scheduled': True,
            'profile': scheduled['profile_filename'],
            'start_time': scheduled['start_time'],
            'start_time_iso': scheduled['start_time_iso'],
            'seconds_until_start': scheduled['seconds_until_start']
        })
    else:
        send_json_response(conn, {'scheduled': False})

def handle_api_cancel_scheduled(conn):
    """POST /api/scheduled/cancel - Cancel scheduled profile"""
    command = CommandMessage.cancel_scheduled()
    
    if QueueHelper.put_nowait(command_queue, command):
        print("[Web Server] Cancelled scheduled profile")
        send_json_response(conn, {
            'success': True,
            'message': 'Cancelled scheduled profile'
        })
    else:
        send_json_response(conn, {
            'success': False,
            'error': 'Command queue full'
        }, 500)

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
    if directory not in ['profiles', 'logs']:
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
        import os
        os.remove(filepath)
    except OSError:
        pass

# Active downloads/uploads, tracked so a starting firing can abort them mid-flight.
_transfer_tasks = set()

async def _supervised_transfer(coro):
    task = asyncio.create_task(coro)
    _transfer_tasks.add(task)
    try:
        await asyncio.wait_for(task, FILE_TRANSFER_TIMEOUT)
    except asyncio.TimeoutError:
        print(f"[Web Server] File transfer exceeded {FILE_TRANSFER_TIMEOUT}s; slot reclaimed")
    except asyncio.CancelledError:
        print("[Web Server] File transfer aborted: kiln started firing")
    finally:
        _transfer_tasks.discard(task)

def cancel_file_transfers():
    for task in list(_transfer_tasks):
        task.cancel()

# === File Management Handlers ===

def handle_api_files_list(conn, directory):
    """GET /api/files/<directory> - List files in directory"""
    try:
        # Check if IDLE
        is_idle, error = check_idle_state()
        if not is_idle:
            send_json_response(conn, error, 403)
            return
        
        # Validate directory
        is_valid, dir_path, error = validate_directory(directory)
        if not is_valid:
            send_json_response(conn, error, 400)
            return
        
        # List files
        import os
        try:
            files = []
            for filename in os.listdir(dir_path):
                filepath = f"{dir_path}/{filename}"
                try:
                    stat = os.stat(filepath)
                    files.append({
                        'name': filename,
                        'size': stat[6],  # st_size
                        'modified': stat[8]  # st_mtime
                    })
                except:
                    # If stat fails, just add name
                    files.append({'name': filename, 'size': 0, 'modified': 0})
            
            send_json_response(conn, {
                'success': True,
                'directory': directory,
                'files': files,
                'count': len(files)
            })
        except OSError as e:
            send_json_response(conn, {
                'success': False,
                'error': f'Failed to list directory: {e}'
            }, 500)
    
    except Exception as e:
        print(f"[Web Server] Error listing files: {e}")
        send_json_response(conn, {'success': False, 'error': str(e)}, 500)

async def handle_api_files_get(conn, directory, filename):
    """GET /api/files/<directory>/<filename> - stream raw file content in 1KB chunks."""
    is_idle, error = check_idle_state()
    if not is_idle:
        send_json_response(conn, error, 403)
        return

    is_valid, dir_path, error = validate_directory(directory)
    if not is_valid:
        send_json_response(conn, error, 400)
        return

    if not safe_filename(filename):
        send_json_response(conn, {'success': False, 'error': 'Invalid filename'}, 400)
        return

    import os
    filepath = f"{dir_path}/{filename}"
    try:
        size = os.stat(filepath)[6]
    except OSError:
        send_response(conn, 404, b'File not found', 'text/plain')
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

    # Non-blocking so drain() suspends on the event loop instead of freezing Core 2.
    gc.collect()
    conn.setblocking(False)
    swriter = asyncio.StreamWriter(conn, {})
    buf = bytearray(FILE_CHUNK_SIZE)
    mv = memoryview(buf)
    try:
        swriter.write(header)
        await swriter.drain()
        with open(filepath, 'rb') as f:
            while True:
                n = f.readinto(buf)
                if not n:
                    break
                swriter.write(mv[:n])
                await swriter.drain()
    except Exception as e:
        # Real disconnect only; CancelledError (timeout/firing) is a BaseException and must reach the supervisor.
        print(f"[Web Server] Download {filename} interrupted: {e}")
    finally:
        gc.collect()

def handle_api_files_delete(conn, directory, filename):
    """DELETE /api/files/<directory>/<filename> - Delete a file"""
    try:
        # Check if IDLE
        is_idle, error = check_idle_state()
        if not is_idle:
            send_json_response(conn, error, 403)
            return
        
        # Validate directory
        is_valid, dir_path, error = validate_directory(directory)
        if not is_valid:
            send_json_response(conn, error, 400)
            return
        
        # Validate filename
        if not safe_filename(filename):
            send_json_response(conn, {
                'success': False,
                'error': 'Invalid filename'
            }, 400)
            return
        
        # Delete file
        filepath = f"{dir_path}/{filename}"
        try:
            import os
            os.remove(filepath)
            print(f"[Web Server] Deleted file: {filepath}")
            send_json_response(conn, {
                'success': True,
                'message': f'Deleted {filename}'
            })
        except OSError as e:
            send_json_response(conn, {
                'success': False,
                'error': f'Failed to delete file: {e}'
            }, 500)
    
    except Exception as e:
        print(f"[Web Server] Error deleting file: {e}")
        send_json_response(conn, {'success': False, 'error': str(e)}, 500)

async def handle_api_files_upload(conn, directory, filename, partial_body, content_length):
    """PUT /api/files/<directory>/<filename> - stream the raw request body to disk in 1KB chunks."""
    is_idle, error = check_idle_state()
    if not is_idle:
        send_json_response(conn, error, 403)
        return

    is_valid, dir_path, error = validate_directory(directory)
    if not is_valid:
        send_json_response(conn, error, 400)
        return

    if not safe_filename(filename):
        send_json_response(conn, {'success': False, 'error': 'Invalid filename'}, 400)
        return

    if content_length <= 0:
        send_json_response(conn, {'success': False, 'error': 'Missing or invalid Content-Length'}, 400)
        return
    if content_length > MAX_UPLOAD_SIZE:
        send_json_response(conn, {
            'success': False,
            'error': f'File too large: {content_length} bytes (max {MAX_UPLOAD_SIZE})'
        }, 413)
        return

    filepath = f"{dir_path}/{filename}"
    gc.collect()
    conn.setblocking(False)
    sreader = asyncio.StreamReader(conn)
    buf = bytearray(FILE_CHUNK_SIZE)
    mv = memoryview(buf)

    written = 0
    try:
        with open(filepath, 'wb') as f:
            # The first body bytes arrived with the headers; write them before reading more.
            if partial_body:
                first = partial_body if len(partial_body) <= content_length else partial_body[:content_length]
                f.write(first)
                written = len(first)
            while written < content_length:
                to_read = min(content_length - written, FILE_CHUNK_SIZE)
                n = await sreader.readinto(mv[:to_read])
                if not n:
                    break
                f.write(mv[:n])
                written += n
    except asyncio.CancelledError:
        # Timeout or firing: drop the partial file, let the supervisor see the cancel.
        _remove_quietly(filepath)
        raise
    except Exception as e:
        _remove_quietly(filepath)
        conn.settimeout(5.0)
        try:
            send_json_response(conn, {'success': False, 'error': f'Failed to write file: {e}'}, 500)
        except Exception:
            pass
        return

    # sendall() is undefined on a non-blocking socket; restore blocking before replying.
    conn.settimeout(5.0)
    gc.collect()

    if written < content_length:
        _remove_quietly(filepath)
        try:
            send_json_response(conn, {'success': False, 'error': 'Upload incomplete: client stopped sending'}, 408)
        except Exception:
            pass
        return

    if directory == 'profiles':
        from server.profile_cache import get_profile_cache
        get_profile_cache().refresh()

    print(f"[Web Server] Uploaded file: {filepath} ({written} bytes)")
    send_json_response(conn, {
        'success': True,
        'message': f'Uploaded {filename}',
        'filename': filename,
        'size': written
    })

def handle_api_files_delete_all(conn, directory):
    """DELETE /api/files/<directory>/all - Delete all files in directory"""
    try:
        # Check if IDLE
        is_idle, error = check_idle_state()
        if not is_idle:
            send_json_response(conn, error, 403)
            return
        
        # Only allow for logs directory
        if directory != 'logs':
            send_json_response(conn, {
                'success': False,
                'error': 'Bulk delete only allowed for logs directory'
            }, 403)
            return
        
        # Delete all files
        import os
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
            
            send_json_response(conn, response)
        
        except OSError as e:
            send_json_response(conn, {
                'success': False,
                'error': f'Failed to delete files: {e}'
            }, 500)
    
    except Exception as e:
        print(f"[Web Server] Error deleting all files: {e}")
        send_json_response(conn, {'success': False, 'error': str(e)}, 500)

# === Status Handlers ===

def handle_api_status(conn):
    """GET /api/status - Get detailed kiln status with PID stats"""
    # Return cached status from control thread
    status = get_status_receiver().get_status()
    send_json_response(conn, status)

# === Tuning Handlers ===

def handle_api_tuning_start(conn, body):
    """POST /api/tuning/start - Start PID auto-tuning"""
    try:
        data = json.loads(body.decode())
        mode = data.get('mode', MODE_STANDARD)
        max_temp = data.get('max_temp')  # None = use mode default

        # Validate mode
        valid_modes = [MODE_SAFE, MODE_STANDARD, MODE_THOROUGH, MODE_HIGH_TEMP]
        if mode not in valid_modes:
            send_json_response(conn, {
                'success': False,
                'error': f'Invalid mode. Must be one of: {", ".join(valid_modes)}'
            }, 400)
            return

        # Validate max_temp if provided
        if max_temp is not None:
            if max_temp < 50 or max_temp > 500:
                send_json_response(conn, {
                    'success': False,
                    'error': 'Maximum temperature must be between 50°C and 500°C'
                }, 400)
                return

        # Send tuning command to control thread
        command = CommandMessage.start_tuning(mode=mode, max_temp=max_temp)

        if QueueHelper.put_nowait(command_queue, command):
            print(f"[Web Server] Started tuning (mode: {mode}, max_temp: {max_temp}°C)")
            cancel_file_transfers()
            send_json_response(conn, {
                'success': True,
                'message': f'Tuning started in {mode} mode'
            })
        else:
            send_json_response(conn, {
                'success': False,
                'error': 'Command queue full, please retry'
            }, 500)

    except Exception as e:
        print(f"[Web Server] Error starting tuning: {e}")
        send_json_response(conn, {'success': False, 'error': str(e)}, 400)

def handle_api_tuning_stop(conn):
    """POST /api/tuning/stop - Stop PID auto-tuning"""
    command = CommandMessage.stop_tuning()

    if QueueHelper.put_nowait(command_queue, command):
        print("[Web Server] Tuning stop requested")
        send_json_response(conn, {'success': True, 'message': 'Tuning stopped'})
    else:
        send_json_response(conn, {
            'success': False,
            'error': 'Command queue full, please retry'
        }, 500)

def handle_api_tuning_status(conn):
    """GET /api/tuning/status - Get tuning status"""
    # Return cached status (includes tuning info if in TUNING state)
    status = get_status_receiver().get_status()
    send_json_response(conn, status)

def handle_tuning_page(conn):
    """Serve tuning.html page"""
    # MEMORY OPTIMIZED: Force garbage collection before building large response
    gc.collect()

    # PERFORMANCE: Use cached HTML instead of blocking file I/O
    from server.html_cache import get_html_cache
    html = get_html_cache().get('tuning')

    if html:
        send_html_response(conn, html)
    else:
        # Fallback: cache miss
        send_response(conn, 404, b'Tuning page not found', 'text/plain')

# === Static File Handlers ===

async def handle_index(conn):
    """Serve pre-rendered index.html (profiles list already included)"""
    # PERFORMANCE: Use pre-rendered HTML from cache (no JSON building, no replacements)
    from server.html_cache import get_html_cache
    html = get_html_cache().get('index')

    if html:
        # Send pre-rendered HTML - client will fetch data via /api/status
        send_html_response(conn, html)
    else:
        # Fallback: cache miss (shouldn't happen if preload succeeded)
        send_response(conn, 500, b'HTML cache miss', 'text/plain')

# === Request Router ===

async def handle_client(conn, addr):
    """Handle individual client connection"""
    global active_connections
    active_connections += 1

    try:
        # Keep socket non-blocking and add timeout
        conn.setblocking(False)
        conn.settimeout(5.0)  # 5 second timeout for recv

        # Try to receive data with timeout
        try:
            req = conn.recv(4096)
        except OSError as e:
            # Timeout or no data available
            print(f"[Web Server] Timeout/error receiving from {addr}: {e}")
            return

        if not req:
            # Client disconnected
            return

        print(f"Request from {addr}")

        # Parse request (initial)
        method, path, headers, body = parse_request(req)
        print(f"{method} {path}")
        
        # PUT (file upload) bodies are NOT buffered here: handle_api_files_upload
        # streams them straight to disk to keep peak RAM ~1KB. Only the partial
        # body already pulled off the socket with the headers is passed along.

        # Handle CORS preflight requests
        if method == 'OPTIONS':
            # Respond to preflight with 200 OK and CORS headers (already included in send_response)
            send_response(conn, 200, b'', 'text/plain')
            return

        # Route request
        if path == '/' or path == '/index.html':
            await handle_index(conn)

        elif path == '/tuning' or path == '/tuning.html':
            handle_tuning_page(conn)

        elif path == '/api/status':
            handle_api_status(conn)

        elif path == '/api/shutdown':
            handle_api_shutdown(conn)

        # Control commands
        elif path == '/api/run':
            if method == 'POST':
                handle_api_run(conn, body)
            else:
                send_response(conn, 405, b'Method not allowed', 'text/plain')

        elif path == '/api/stop':
            if method == 'POST':
                handle_api_stop(conn)
            else:
                send_response(conn, 405, b'Method not allowed', 'text/plain')

        elif path == '/api/clear-error':
            if method == 'POST':
                handle_api_clear_error(conn)
            else:
                send_response(conn, 405, b'Method not allowed', 'text/plain')

        elif path == '/api/reboot':
            if method == 'POST':
                handle_api_reboot(conn)
            else:
                send_response(conn, 405, b'Method not allowed', 'text/plain')

        # Tuning endpoints
        elif path == '/api/tuning/start':
            if method == 'POST':
                handle_api_tuning_start(conn, body)
            else:
                send_response(conn, 405, b'Method not allowed', 'text/plain')

        elif path == '/api/tuning/stop':
            if method == 'POST':
                handle_api_tuning_stop(conn)
            else:
                send_response(conn, 405, b'Method not allowed', 'text/plain')

        elif path == '/api/tuning/status':
            if method == 'GET':
                handle_api_tuning_status(conn)
            else:
                send_response(conn, 405, b'Method not allowed', 'text/plain')

        # Scheduling endpoints
        elif path == '/api/schedule':
            if method == 'POST':
                handle_api_schedule(conn, body)
            else:
                send_response(conn, 405, b'Method not allowed', 'text/plain')

        elif path == '/api/scheduled':
            if method == 'GET':
                handle_api_scheduled_status(conn)
            else:
                send_response(conn, 405, b'Method not allowed', 'text/plain')

        elif path == '/api/scheduled/cancel':
            if method == 'POST':
                handle_api_cancel_scheduled(conn)
            else:
                send_response(conn, 405, b'Method not allowed', 'text/plain')

        # File management endpoints
        elif path.startswith('/api/files/'):
            # Parse path: /api/files/<directory> or /api/files/<directory>/<filename>
            parts = path.split('/')
            if len(parts) == 4:
                # /api/files/<directory>
                directory = parts[3]
                if method == 'GET':
                    handle_api_files_list(conn, directory)
                else:
                    send_response(conn, 405, b'Method not allowed', 'text/plain')
            
            elif len(parts) == 5:
                # /api/files/<directory>/<filename>
                directory = parts[3]
                filename = parts[4]
                
                if filename == 'all':
                    # DELETE /api/files/<directory>/all
                    if method == 'DELETE':
                        handle_api_files_delete_all(conn, directory)
                    else:
                        send_response(conn, 405, b'Method not allowed', 'text/plain')
                else:
                    # GET, PUT, or DELETE /api/files/<directory>/<filename>
                    if method == 'GET':
                        await _supervised_transfer(handle_api_files_get(conn, directory, filename))
                    elif method == 'PUT':
                        try:
                            content_length = int(headers.get('content-length', 0))
                        except (ValueError, TypeError):
                            content_length = 0
                        await _supervised_transfer(handle_api_files_upload(conn, directory, filename, body, content_length))
                    elif method == 'DELETE':
                        handle_api_files_delete(conn, directory, filename)
                    else:
                        send_response(conn, 405, b'Method not allowed', 'text/plain')
            else:
                send_response(conn, 404, b'Not found', 'text/plain')

        else:
            send_response(conn, 404, b'Not found', 'text/plain')

    except Exception as e:
        print(f"Error handling request: {e}")
        try:
            send_response(conn, 500, f'Server error: {e}'.encode(), 'text/plain')
        except:
            pass

    finally:
        active_connections -= 1
        try:
            conn.close()
        except:
            pass

async def start_server(cmd_queue):
    """
    Start the HTTP server with non-blocking socket

    Args:
        cmd_queue: ThreadSafeQueue for sending commands to control thread

    Note:
        Status updates are handled by StatusReceiver singleton, which should
        be initialized and started separately in main.py
    """
    global command_queue
    command_queue = cmd_queue

    print(f"[Web Server] Starting HTTP server on port {config.WEB_SERVER_PORT}")

    # Create server socket
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind(('', config.WEB_SERVER_PORT))
    s.listen(5)
    s.setblocking(False)

    print("[Web Server] HTTP server listening!")

    socket_error_count = 0
    max_socket_errors = MAX_SOCKET_ERRORS  # Allow 5 seconds of errors

    while True:
        try:
            # Check if we're at connection limit before accepting
            if active_connections >= MAX_CONCURRENT_CONNECTIONS:
                await asyncio.sleep(SERVER_LOOP_INTERVAL)
                continue

            conn, addr = s.accept()
            socket_error_count = 0  # Reset on successful accept
            # Handle each client in a separate task
            asyncio.create_task(handle_client(conn, addr))

        except OSError as e:
            # Check if it's a "no connection" error (errno 11 EAGAIN) vs real error
            if hasattr(e, 'args') and len(e.args) > 0 and e.args[0] == 11:
                # EAGAIN - no connection available (normal for non-blocking socket)
                pass
            else:
                # Real socket error - count and potentially recover
                socket_error_count += 1
                if socket_error_count <= MAX_SOCKET_ERROR_PRINT:  # Only print first few errors to avoid spam
                    print(f"[Web Server] Socket error ({socket_error_count}/{max_socket_errors}): {e}")

                if socket_error_count >= max_socket_errors:
                    print("[Web Server] Too many socket errors - attempting server restart...")
                    try:
                        # Try to restart the server socket
                        s.close()
                        s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
                        s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                        s.bind(('', config.WEB_SERVER_PORT))
                        s.listen(5)
                        s.setblocking(False)
                        socket_error_count = 0
                        print("[Web Server] Server socket restarted successfully")
                    except Exception as restart_error:
                        print(f"[Web Server] Server restart failed: {restart_error}")
                        print("[Web Server] Giving up on web server - Core 1 continues")
                        return  # Exit web server but let Core 1 continue

        await asyncio.sleep(SERVER_LOOP_INTERVAL)
