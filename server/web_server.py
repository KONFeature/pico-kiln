# server/web_server.py
# HTTP server for monitoring and control interface
#
# This module runs on Core 2 and communicates with the control thread (Core 1)
# via thread-safe queues. It never directly accesses hardware.

import asyncio
import json
import socket
import gc
import config
from kiln.comms import CommandMessage, QueueHelper
from kiln.tuner import MODE_SAFE, MODE_STANDARD, MODE_THOROUGH, MODE_HIGH_TEMP
from server.status_receiver import get_status_receiver

# HTTP response templates
HTTP_200 = "HTTP/1.1 200 OK\r\n"
HTTP_404 = "HTTP/1.1 404 Not Found\r\n"
HTTP_500 = "HTTP/1.1 500 Internal Server Error\r\n"

# Global communication channels (initialized in start_server)
command_queue = None

# Connection limiting to prevent memory exhaustion
active_connections = 0
MAX_CONCURRENT_CONNECTIONS = 2  # Limit to 2 concurrent connections on Pico

# Request size limits
MAX_PROFILE_SIZE = 10240  # 10KB max for profile uploads

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
    """Send HTTP response"""
    status_text = {200: 'OK', 404: 'Not Found', 500: 'Error'}
    conn.send(f'HTTP/1.1 {status} {status_text.get(status, "Unknown")}\n'.encode())
    conn.send(f'Content-Type: {content_type}\n'.encode())
    conn.send(b'Connection: close\n\n')
    conn.sendall(body)

def send_json_response(conn, data, status=200):
    """Send JSON response"""
    json_data = json.dumps(data)
    send_response(conn, status, json_data.encode(), 'application/json')

def send_html_response(conn, html, status=200):
    """Send HTML response"""
    send_response(conn, status, html.encode() if isinstance(html, str) else html, 'text/html')

# === API Handlers ===

def handle_api_state(conn):
    """GET /api/state - Return current system state (legacy endpoint)"""
    # MEMORY OPTIMIZED: Use get_fields() to avoid full status copy
    receiver = get_status_receiver()
    cached = receiver.status_cache.get_fields(
        'ssr_output', 'current_temp', 'target_temp', 'profile_name', 'state'
    )

    response = {
        "ssr_state": cached.get('ssr_output', 0.0) > 0,  # True if SSR has any output
        "current_temp": cached.get('current_temp', 0.0),
        "target_temp": cached.get('target_temp', 0.0),
        "current_program": cached.get('profile_name'),
        "program_running": cached.get('state') == "RUNNING"
    }
    send_json_response(conn, response)

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

def handle_api_info(conn):
    """GET /api/info - Return system info"""
    # Import here to avoid circular dependency
    import network

    # Get IP address from WiFi interface
    wlan = network.WLAN(network.STA_IF)
    ip_address = wlan.ifconfig()[0] if wlan.isconnected() else "Not connected"

    info = {
        "name": "Pico Kiln Controller",
        "version": "0.2.0",
        "hardware": "Raspberry Pi Pico 2 (Dual Core)",
        "sensor": "MAX31856",
        "architecture": "Multi-threaded (Core 1: Control, Core 2: Web)",
        "ip_address": ip_address
    }
    send_json_response(conn, info)

# === Profile Management Handlers ===

def handle_api_profile_get(conn, profile_name):
    """GET /api/profile/<name> - Get specific profile"""
    try:
        # Verify profile exists in cache first (fast check)
        from server.profile_cache import get_profile_cache
        if not get_profile_cache().exists(profile_name):
            send_json_response(conn, {'success': False, 'error': 'Profile not found'}, 404)
            return

        # Read profile data from disk (acceptable since downloads are infrequent)
        import os
        filename = f"{config.PROFILES_DIR}/{profile_name}.json"

        with open(filename, 'r') as f:
            profile_data = json.load(f)

        send_json_response(conn, {'profile': profile_data, 'success': True})
    except Exception as e:
        send_json_response(conn, {'success': False, 'error': str(e)}, 500)

def handle_api_profile_upload(conn, body):
    """POST /api/profile - Upload new profile"""
    try:
        from kiln.profile import Profile
        import os

        # Check profile size limit to prevent memory exhaustion
        if len(body) > MAX_PROFILE_SIZE:
            send_json_response(conn, {
                'success': False,
                'error': f'Profile too large (max {MAX_PROFILE_SIZE} bytes)'
            }, 400)
            return

        profile_data = json.loads(body.decode())

        # Validate profile by trying to create it
        profile = Profile(profile_data)

        # Save to file
        filename = f"{config.PROFILES_DIR}/{profile.name}.json"

        # Create profiles directory if it doesn't exist
        try:
            os.mkdir(config.PROFILES_DIR)
        except:
            pass

        with open(filename, 'w') as f:
            json.dump(profile_data, f)

        # Add filename to cache so it appears in profile list immediately
        from server.profile_cache import get_profile_cache
        get_profile_cache().add(profile.name)

        print(f"Profile '{profile.name}' uploaded")
        send_json_response(conn, {'success': True, 'message': f'Profile {profile.name} saved'})
    except Exception as e:
        print(f"Error uploading profile: {e}")
        send_json_response(conn, {'success': False, 'error': str(e)}, 400)

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
    # PERFORMANCE: Use cached HTML instead of blocking file I/O
    from server.html_cache import get_html_cache
    html = get_html_cache().get('tuning')

    if html:
        send_html_response(conn, html)
        gc.collect()  # Free memory after large HTML serving
    else:
        # Fallback: cache miss
        send_response(conn, 404, b'Tuning page not found', 'text/plain')

# === Static File Handlers ===

async def handle_index(conn):
    """Serve index.html with current state"""
    try:
        # PERFORMANCE: Use cached HTML instead of blocking file I/O
        from server.html_cache import get_html_cache
        html = get_html_cache().get('index')

        if not html:
            # Fallback: cache miss (shouldn't happen if preload succeeded)
            send_response(conn, 500, b'HTML cache miss', 'text/plain')
            return

        # Yield to event loop immediately after getting cache
        await asyncio.sleep(0)

        # MEMORY OPTIMIZED: Use get_fields() to fetch only needed fields
        receiver = get_status_receiver()
        cached = receiver.status_cache.get_fields(
            'ssr_output', 'current_temp', 'target_temp', 'profile_name', 'state'
        )

        # Yield before building profiles list
        await asyncio.sleep(0)

        # Build profiles list HTML (using list + join for memory efficiency)
        # PERFORMANCE: Use cached profile list instead of blocking os.listdir()
        from server.profile_cache import get_profile_cache
        profiles_parts = ['<ul>']

        profile_names = get_profile_cache().list_profiles()

        if profile_names:
            for profile_name in profile_names:
                profiles_parts.append(f'<li>{profile_name} <button onclick="startProfile(\'{profile_name}\')">Start</button></li>')
        else:
            profiles_parts.append('<li>No profiles found</li>')

        profiles_parts.append('</ul>')
        profiles_html = ''.join(profiles_parts)

        # Yield before JSON serialization
        await asyncio.sleep(0)

        # MEMORY OPTIMIZED: Build single JSON object for client-side rendering
        # This reduces 8 string.replace() calls to just 2, saving ~10KB in temporary allocations
        ssr_output = cached.get('ssr_output', 0.0)
        status_data = {
            'status': f'{ssr_output:.0f}%' if ssr_output > 0 else 'OFF',
            'current_temp': cached.get('current_temp', 0.0),
            'target_temp': cached.get('target_temp', 0.0),
            'program': cached.get('profile_name') or 'None',
            'state': cached.get('state', 'IDLE')
        }

        # MEMORY OPTIMIZED: Combine replacements to reduce string allocations
        html = html.replace('{DATA}', json.dumps(status_data)).replace('{profiles_list}', profiles_html)

        # Yield before sending response
        await asyncio.sleep(0)

        send_html_response(conn, html)
        gc.collect()  # MEMORY OPTIMIZED: Free memory after HTML generation
    except OSError:
        # File not found, serve simple fallback
        html = """<!DOCTYPE html>
<html>
<head>
    <title>Pico Kiln Controller</title>
    <style>
        body { font-family: Arial, sans-serif; margin: 40px; }
        h1 { color: #333; }
        .status { background: #f0f0f0; padding: 20px; border-radius: 5px; }
    </style>
</head>
<body>
    <h1>Pico Kiln Controller</h1>
    <div class="status">
        <h2>Status</h2>
        <p>System is running!</p>
        <p><a href="/api/state">View State API</a></p>
        <p><a href="/api/info">View Info API</a></p>
    </div>
</body>
</html>"""
        send_html_response(conn, html)

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

        # Parse request
        method, path, headers, body = parse_request(req)
        print(f"{method} {path}")

        # Route request
        if path == '/' or path == '/index.html':
            await handle_index(conn)

        elif path == '/tuning' or path == '/tuning.html':
            handle_tuning_page(conn)

        elif path == '/api/state':
            handle_api_state(conn)

        elif path == '/api/status':
            handle_api_status(conn)

        elif path == '/api/info':
            handle_api_info(conn)

        elif path == '/api/shutdown':
            handle_api_shutdown(conn)

        # Profile management
        elif path == '/api/profile':
            if method == 'POST':
                handle_api_profile_upload(conn, body)
            else:
                send_response(conn, 405, b'Method not allowed', 'text/plain')

        elif path.startswith('/api/profile/'):
            # Extract profile name from path: GET /api/profile/<name>
            profile_name = path.split('/')[-1]
            if method == 'GET':
                handle_api_profile_get(conn, profile_name)
            else:
                send_response(conn, 405, b'Method not allowed', 'text/plain')

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

    while True:
        try:
            # Check if we're at connection limit before accepting
            if active_connections >= MAX_CONCURRENT_CONNECTIONS:
                await asyncio.sleep(0.1)
                continue

            conn, addr = s.accept()
            # Handle each client in a separate task
            asyncio.create_task(handle_client(conn, addr))
        except OSError:
            # No connection available, yield to other tasks
            pass

        await asyncio.sleep(0.1)
