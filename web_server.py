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
    """GET /api/state - Return current system state"""
    response = {
        "ssr_state": state.ssr_pin.value() if state.ssr_pin else False,
        "current_temp": state.current_temp,
        "target_temp": state.target_temp,
        "current_program": state.current_program,
        "program_running": state.program_running
    }
    send_json_response(conn, response)

def handle_api_shutdown(conn):
    """POST /api/shutdown - Emergency shutdown: turn off SSR and stop program"""
    # Turn off SSR
    if state.ssr_pin:
        state.ssr_pin.off()

    # Stop current program
    state.program_running = False
    state.current_program = None
    state.target_temp = 0.0

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

# === Static File Handlers ===

def handle_index(conn):
    """Serve index.html with current state"""
    try:
        with open("static/index.html", "r") as f:
            html = f.read()

        # Replace template variables
        ssr_status = 'ON' if (state.ssr_pin and state.ssr_pin.value()) else 'OFF'
        status_color = '#4CAF50' if (state.ssr_pin and state.ssr_pin.value()) else '#f44336'

        html = html.replace('{status}', ssr_status)
        html = html.replace('{status_color}', status_color)
        html = html.replace('{current_temp}', f'{state.current_temp:.1f}')
        html = html.replace('{target_temp}', f'{state.target_temp:.1f}')
        html = html.replace('{program}', state.current_program or 'None')

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

        elif path == '/api/info':
            handle_api_info(conn)

        elif path == '/api/shutdown':
            handle_api_shutdown(conn)

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
