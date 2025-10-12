# web_server.py
# HTTP server for monitoring and control interface

import asyncio
import json
import socket
import config

# HTTP response templates
HTTP_200 = "HTTP/1.1 200 OK\r\n"
HTTP_404 = "HTTP/1.1 404 Not Found\r\n"
HTTP_500 = "HTTP/1.1 500 Internal Server Error\r\n"

# Global reference to state (passed from main)
state = None

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
    response = {
        "ssr_state": state.ssr_pin.value() if state.ssr_pin else False,
        "current_temp": state.controller.current_temp,
        "target_temp": state.controller.target_temp,
        "current_program": state.controller.active_profile.name if state.controller.active_profile else None,
        "program_running": state.controller.state == "RUNNING"
    }
    send_json_response(conn, response)

def handle_api_shutdown(conn):
    """POST /api/shutdown - Emergency shutdown: turn off SSR and stop program"""
    # Stop controller
    state.controller.stop()

    # Force SSR off
    state.ssr_controller.force_off()

    print("Emergency shutdown triggered via API")

    response = {
        "success": True,
        "message": "System shutdown: SSR off, program stopped"
    }
    send_json_response(conn, response)

def handle_api_info(conn):
    """GET /api/info - Return system info"""
    info = {
        "name": "Pico Kiln Controller",
        "version": "0.1.0",
        "hardware": "Raspberry Pi Pico 2",
        "sensor": "MAX31856",
        "ip_address": state.ip_address
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
        from kiln.profile import Profile

        data = json.loads(body.decode())
        profile_name = data.get('profile')

        if not profile_name:
            send_json_response(conn, {'success': False, 'error': 'Profile name required'}, 400)
            return

        # Load profile
        filename = f"{config.PROFILES_DIR}/{profile_name}.json"
        profile = Profile.load_from_file(filename)

        # Start kiln
        state.controller.run_profile(profile)

        send_json_response(conn, {
            'success': True,
            'message': f'Started profile: {profile.name}'
        })
    except Exception as e:
        print(f"Error starting profile: {e}")
        send_json_response(conn, {'success': False, 'error': str(e)}, 400)

def handle_api_stop(conn):
    """POST /api/stop - Stop current profile"""
    try:
        state.controller.stop()
        state.ssr_controller.force_off()
        send_json_response(conn, {'success': True, 'message': 'Profile stopped'})
    except Exception as e:
        send_json_response(conn, {'success': False, 'error': str(e)}, 400)

# === Status Handlers ===

def handle_api_status(conn):
    """GET /api/status - Get detailed kiln status with PID stats"""
    try:
        status = state.controller.get_status()

        # Add PID statistics
        status['pid_stats'] = state.pid.get_stats()

        # Add SSR state
        status['ssr_state'] = state.ssr_controller.get_state()

        send_json_response(conn, status)
    except Exception as e:
        send_json_response(conn, {'success': False, 'error': str(e)}, 500)

# === Static File Handlers ===

def handle_index(conn):
    """Serve index.html with current state"""
    try:
        import os
        with open("static/index.html", "r") as f:
            html = f.read()

        # Replace template variables - SSR status
        ssr_status = 'ON' if (state.ssr_pin and state.ssr_pin.value()) else 'OFF'
        status_color = '#4CAF50' if (state.ssr_pin and state.ssr_pin.value()) else '#f44336'

        # Controller state
        controller_state = str(state.controller.state)
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
        html = html.replace('{current_temp}', f'{state.controller.current_temp:.1f}')
        html = html.replace('{target_temp}', f'{state.controller.target_temp:.1f}')
        html = html.replace('{program}', state.controller.active_profile.name if state.controller.active_profile else 'None')
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

async def start_server(app_state):
    """Start the HTTP server with non-blocking socket"""
    global state
    state = app_state

    print(f"Starting HTTP server on port {config.WEB_SERVER_PORT}")

    # Create server socket
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind(('', config.WEB_SERVER_PORT))
    s.listen(5)
    s.setblocking(False)

    print("HTTP server listening!")

    while True:
        try:
            conn, addr = s.accept()
            # Handle each client in a separate task
            asyncio.create_task(handle_client(conn, addr))
        except OSError:
            # No connection available, yield to other tasks
            pass

        await asyncio.sleep(0.1)
