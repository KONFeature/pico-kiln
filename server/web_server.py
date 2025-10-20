# server/web_server.py
# HTTP server for monitoring and control interface
#
# This module runs on Core 2 and communicates with the control thread (Core 1)
# via thread-safe queues. It never directly accesses hardware.

import asyncio
import json
import socket
import config
from kiln.comms import CommandMessage, QueueHelper
from server.status_receiver import get_status_receiver

# HTTP response templates
HTTP_200 = "HTTP/1.1 200 OK\r\n"
HTTP_404 = "HTTP/1.1 404 Not Found\r\n"
HTTP_500 = "HTTP/1.1 500 Internal Server Error\r\n"

# Global communication channels (initialized in start_server)
command_queue = None

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
    cached = get_status_receiver().get_status()

    response = {
        "ssr_state": cached.get('ssr_is_on', False),
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
        from kiln.profile import Profile
        import os

        # Sanitize filename
        filename = f"{config.PROFILES_DIR}/{profile_name}.json"
        if not os.path.exists(filename):
            send_json_response(conn, {'success': False, 'error': 'Profile not found'}, 404)
            return

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

        # Verify profile exists before sending command
        import os
        filename = f"{config.PROFILES_DIR}/{profile_name}.json"
        if not os.path.exists(filename):
            send_json_response(conn, {'success': False, 'error': f'Profile not found: {profile_name}'}, 404)
            return

        # Send command to control thread (Core 1 will load the profile)
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
        target_temp = data.get('target_temp', 200)

        # Validate target temperature
        if target_temp < 50 or target_temp > 500:
            send_json_response(conn, {
                'success': False,
                'error': 'Target temperature must be between 50°C and 500°C'
            }, 400)
            return

        # Send tuning command to control thread
        command = CommandMessage.start_tuning(target_temp)

        if QueueHelper.put_nowait(command_queue, command):
            print(f"[Web Server] Started tuning (target: {target_temp}°C)")
            send_json_response(conn, {
                'success': True,
                'message': f'Tuning started with target {target_temp}°C'
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
    try:
        with open("static/tuning.html", "r") as f:
            html = f.read()
        send_html_response(conn, html)
    except OSError:
        send_response(conn, 404, b'Tuning page not found', 'text/plain')

# === Static File Handlers ===

def handle_index(conn):
    """Serve index.html with current state"""
    try:
        import os
        with open("static/index.html", "r") as f:
            html = f.read()

        # Get cached status from control thread
        cached = get_status_receiver().get_status()

        # Replace template variables - SSR status
        ssr_status = 'ON' if cached.get('ssr_is_on', False) else 'OFF'
        status_color = '#4CAF50' if cached.get('ssr_is_on', False) else '#f44336'

        # Controller state
        controller_state = cached.get('state', 'IDLE')
        state_class = controller_state.lower()

        # Build profiles list HTML
        profiles_html = '<ul class="profile-list">'
        try:
            # List all JSON files in profiles directory
            profile_files = [f for f in os.listdir(config.PROFILES_DIR) if f.endswith('.json')]

            if profile_files:
                for profile_file in sorted(profile_files):
                    profile_name = profile_file[:-5]  # Remove .json extension
                    profiles_html += f'''
                    <li class="profile-item">
                        <span class="profile-name">{profile_name}</span>
                        <button class="btn-start btn-small" onclick="startProfile('{profile_name}')">Start</button>
                    </li>'''
            else:
                profiles_html += '<li class="empty-state">No profiles found. Upload a profile using the API.</li>'
        except:
            profiles_html += '<li class="empty-state">No profiles directory found.</li>'

        profiles_html += '</ul>'

        # Replace all template variables
        html = html.replace('{status}', ssr_status)
        html = html.replace('{status_color}', status_color)
        html = html.replace('{current_temp}', f'{cached.get("current_temp", 0.0):.1f}')
        html = html.replace('{target_temp}', f'{cached.get("target_temp", 0.0):.1f}')
        html = html.replace('{program}', cached.get('profile_name') or 'None')
        html = html.replace('{state}', controller_state)
        html = html.replace('{state_class}', state_class)
        html = html.replace('{profiles_list}', profiles_html)

        send_html_response(conn, html)
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
    try:
        # Set socket to blocking for recv
        conn.setblocking(True)
        req = conn.recv(4096)
        print(f"Request from {addr}")

        # Parse request
        method, path, headers, body = parse_request(req)
        print(f"{method} {path}")

        # Route request
        if path == '/' or path == '/index.html':
            handle_index(conn)

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
            conn, addr = s.accept()
            # Handle each client in a separate task
            asyncio.create_task(handle_client(conn, addr))
        except OSError:
            # No connection available, yield to other tasks
            pass

        await asyncio.sleep(0.1)
